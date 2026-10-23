//! Serialized per-account JSON-RPC transport. No model/thread APIs are exposed.
use crate::runtime::BackendCommand;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct RpcError {
    pub message: String,
    pub retry_after: Option<u64>,
    pub disconnected: bool,
}
impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for RpcError {}
impl RpcError {
    fn local(s: &str) -> Self {
        Self {
            message: s.into(),
            retry_after: None,
            disconnected: true,
        }
    }
    fn remote(v: &Value) -> Self {
        let code = v["code"].as_i64().unwrap_or(0);
        let raw = v["message"].as_str().unwrap_or("").to_lowercase();
        let message = if code == -32601
            || raw.contains("unknown variant")
            || raw.contains("method not found")
        {
            "官方组件不支持此接口，请更新组件".into()
        } else if raw.contains("keyring")
            || raw.contains("credential store")
            || raw.contains("secret service")
        {
            "系统凭据库不可用；请启用凭据服务，或切换为仅本次会话登录".into()
        } else if raw.contains("401")
            || raw.contains("unauthorized")
            || raw.contains("authentication")
            || raw.contains("not logged in")
        {
            "登录已失效，请重新登录此账号".into()
        } else if raw.contains("429") || raw.contains("rate limit") || code == -32001 {
            "请求过于频繁，已延后重试".into()
        } else {
            format!("官方服务请求失败（代码 {code}），请稍后重试")
        };
        Self {
            message,
            retry_after: v
                .pointer("/data/retryAfterSeconds")
                .and_then(Value::as_u64)
                .or_else(|| {
                    if raw.contains("429") || code == -32001 {
                        Some(60)
                    } else {
                        None
                    }
                }),
            disconnected: false,
        }
    }
}

pub struct Rpc {
    child: Child,
    input: ChildStdin,
    rx: Receiver<Result<Value, ()>>,
    next_id: u64,
    pub notifications: VecDeque<Value>,
    cancel: Arc<AtomicBool>,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
impl Rpc {
    pub fn spawn(
        backend: &BackendCommand,
        home: &Path,
        ephemeral: bool,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, RpcError> {
        crate::store::ensure_private_dir(home).map_err(|_| RpcError::local("无法创建账号目录"))?;
        let mut cmd = Command::new(&backend.executable);
        cmd.args(&backend.prefix)
            .args([
                "--listen",
                "stdio://",
                "-c",
                if ephemeral {
                    "cli_auth_credentials_store=\"ephemeral\""
                } else {
                    "cli_auth_credentials_store=\"keyring\""
                },
                "-c",
                "analytics.enabled=false",
            ])
            .current_dir(home)
            .env("CODEX_HOME", home)
            .env_remove("CODEX_ACCESS_TOKEN")
            .env_remove("CODEX_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd
            .spawn()
            .map_err(|_| RpcError::local("官方组件无法启动，请检查路径和文件权限"))?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut buf = vec![];
                // read_until is bounded explicitly using fill_buf, preventing unlimited allocation.
                loop {
                    let bytes = match reader.fill_buf() {
                        Ok(b) => b,
                        Err(_) => {
                            let _ = tx.send(Err(()));
                            return;
                        }
                    };
                    if bytes.is_empty() {
                        let _ = tx.send(Err(()));
                        return;
                    }
                    let n = bytes
                        .iter()
                        .position(|b| *b == b'\n')
                        .map_or(bytes.len(), |p| p + 1);
                    if buf.len() + n > 1024 * 1024 {
                        let _ = tx.send(Err(()));
                        return;
                    }
                    buf.extend_from_slice(&bytes[..n]);
                    reader.consume(n);
                    if buf.last() == Some(&b'\n') {
                        break;
                    }
                }
                let value = serde_json::from_slice(&buf).map_err(|_| ());
                let invalid = value.is_err();
                if tx.send(value).is_err() || invalid {
                    return;
                }
            }
        });
        #[cfg(windows)]
        let job = make_job(&child);
        let mut rpc = Self {
            child,
            input,
            rx,
            next_id: 1,
            notifications: VecDeque::new(),
            cancel,
            #[cfg(windows)]
            job,
        };
        rpc.request("initialize",json!({"clientInfo":{"name":"quotamonitor","title":"QuotaMonitor","version":env!("CARGO_PKG_VERSION")}}),Duration::from_secs(30))?;
        rpc.write(&json!({"method":"initialized","params":{}}))?;
        Ok(rpc)
    }
    fn write(&mut self, value: &Value) -> Result<(), RpcError> {
        let mut data = serde_json::to_vec(value).map_err(|_| RpcError::local("请求编码失败"))?;
        data.push(b'\n');
        self.input
            .write_all(&data)
            .and_then(|_| self.input.flush())
            .map_err(|_| RpcError::local("官方组件连接已断开"))
    }
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({"id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + timeout;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(RpcError::local("操作已停止"));
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Err(RpcError {
                    message: "请求超时；重置操作将保留原操作编号供核对".into(),
                    retry_after: None,
                    disconnected: false,
                });
            };
            match self.rx.recv_timeout(left.min(Duration::from_millis(100))) {
                Ok(Ok(v)) => {
                    if v.get("method").is_some() {
                        self.route_message(v)?;
                        continue;
                    }
                    if v["id"].as_u64() != Some(id) {
                        continue;
                    }
                    if let Some(error) = v.get("error") {
                        return Err(RpcError::remote(error));
                    }
                    return v
                        .get("result")
                        .cloned()
                        .ok_or_else(|| RpcError::local("官方组件响应无效"));
                }
                Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(RpcError::local("官方组件已退出或响应格式错误"));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
    fn route_message(&mut self, v: Value) -> Result<(), RpcError> {
        if let Some(id) = v.get("id") {
            // This app never starts turns or executes tools. Reject unsolicited requests.
            self.write(&json!({"id":id,"error":{"code":-32601,"message":"Quota monitor does not expose this method"}}))?;
        } else {
            if self.notifications.len() >= 100 {
                self.notifications.pop_front();
            }
            self.notifications.push_back(v);
        }
        Ok(())
    }
    pub fn drain(&mut self) -> Result<(), RpcError> {
        loop {
            match self.rx.try_recv() {
                Ok(Ok(v)) => {
                    if v.get("method").is_some() {
                        self.route_message(v)?;
                    }
                }
                Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(RpcError::local("官方组件已退出"));
                }
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
            }
        }
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        #[cfg(windows)]
        if !self.job.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.job);
            }
        }
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-TERM", "--", &format!("-{}", self.child.id())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[cfg(windows)]
fn make_job(child: &Child) -> windows_sys::Win32::Foundation::HANDLE {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return job;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&info) as u32,
        ) == 0
            || AssignProcessToJobObject(job, child.as_raw_handle().cast()) == 0
        {
            CloseHandle(job);
            return std::ptr::null_mut();
        }
        job
    }
}
