use crate::{
    login::BrowserLogin,
    model::{Quota, now},
    rpc::{Rpc, RpcError},
    runtime::Backend,
    store::{self, PendingReset},
    token::{self, TokenClient},
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    LoggedOut,
    Ready,
    WaitingLogin,
    Syncing,
    Paused,
    Resetting,
    Error,
}
#[derive(Clone, Debug)]
pub struct ViewState {
    pub id: String,
    pub status: Status,
    pub quota: Option<Quota>,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub message: String,
    pub pending: bool,
    pub login_url: Option<String>,
    pub user_code: Option<String>,
    pub next_retry_secs: u64,
}
impl ViewState {
    pub fn new(id: String, cached: Option<Quota>) -> Self {
        Self {
            id,
            status: Status::Connecting,
            quota: cached,
            email: None,
            plan: None,
            message: "连接中".into(),
            pending: false,
            login_url: None,
            user_code: None,
            next_retry_secs: 0,
        }
    }
}
#[derive(Clone)]
pub enum Command {
    Refresh,
    Login { device: bool },
    CancelLogin,
    UseToken(String),
    SwitchCodex(PathBuf),
    SetSharedCodex(Option<PathBuf>),
    Reset,
    Logout,
    Pause(bool),
    Interval(u64),
    Stop,
}
#[derive(Clone, Debug)]
pub enum Event {
    State(ViewState),
    OpenLogin(String),
    Removed(String),
    CodexSwitched {
        id: String,
        result: Result<PathBuf, String>,
    },
}
pub struct Worker {
    pub tx: Sender<Command>,
    cancel: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Worker {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        id: String,
        home: PathBuf,
        backend: impl Into<Backend>,
        interval: u64,
        paused: bool,
        ephemeral: bool,
        events: Sender<Event>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let backend = backend.into();
        let join = thread::spawn(move || {
            Runner {
                id: id.clone(),
                home,
                backend,
                interval,
                paused,
                ephemeral,
                events,
                wake,
                cancel: c,
                rx,
                rpc: None,
                token_client: None,
                browser_login: None,
                shared_auth: None,
                state: ViewState::new(id, None),
                failures: 0,
                next_poll: Instant::now(),
                login_id: None,
                login_started: None,
                bound_account: None,
            }
            .run()
        });
        Self {
            tx,
            cancel,
            join: Some(join),
        }
    }
    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Command::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
struct Runner {
    id: String,
    home: PathBuf,
    backend: Backend,
    interval: u64,
    paused: bool,
    ephemeral: bool,
    events: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Command>,
    rpc: Option<Rpc>,
    token_client: Option<TokenClient>,
    browser_login: Option<BrowserLogin>,
    shared_auth: Option<PathBuf>,
    state: ViewState,
    failures: u32,
    next_poll: Instant,
    login_id: Option<String>,
    login_started: Option<Instant>,
    bound_account: Option<String>,
}
impl Runner {
    fn emit(&self) {
        let _ = self.events.send(Event::State(self.state.clone()));
        (self.wake)();
    }
    fn set_status(&mut self, status: Status, message: impl Into<String>) {
        self.state.status = status;
        self.state.message = message.into();
        self.emit();
    }
    fn connect(&mut self) -> Result<(), RpcError> {
        if self.rpc.is_none() {
            let Backend::AppServer(backend) = &self.backend else {
                return Ok(());
            };
            self.rpc = Some(Rpc::spawn(
                backend,
                &self.home,
                self.ephemeral,
                self.cancel.clone(),
            )?);
        }
        Ok(())
    }
    fn switch_codex(&mut self, destination: PathBuf) {
        let result = (|| -> Result<PathBuf, String> {
            if !self.backend.uses_tokens() {
                return Err("请使用网页登录保存完整账号后再切换 Codex".into());
            }
            if self.token_client.is_none() {
                self.token_client = Some(TokenClient::new(&self.home)?);
                self.token_client
                    .as_mut()
                    .unwrap()
                    .set_shared_auth(self.shared_auth.clone());
            }
            self.token_client
                .as_mut()
                .unwrap()
                .replace_codex_auth(&destination)?;
            self.shared_auth = Some(destination.clone());
            Ok(destination)
        })();
        let _ = self.events.send(Event::CodexSwitched {
            id: self.id.clone(),
            result,
        });
        (self.wake)();
    }
    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        if self.backend.uses_tokens() {
            if self.token_client.is_none() {
                self.token_client = Some(TokenClient::new(&self.home).map_err(|s| logical(&s))?);
                self.token_client
                    .as_mut()
                    .unwrap()
                    .set_shared_auth(self.shared_auth.clone());
            }
            let client = self.token_client.as_mut().unwrap();
            return match method {
                "account/read" => client.account().map_err(|s| logical(&s)),
                "account/rateLimits/read" => client.quota(),
                "account/logout" => {
                    token::forget(&self.home).map_err(|s| logical(&s))?;
                    self.token_client = None;
                    Ok(json!({}))
                }
                _ => Err(logical(
                    "当前使用已有 token 查询额度，请在账号详情导入 token",
                )),
            };
        }
        self.connect()?;
        self.rpc
            .as_mut()
            .unwrap()
            .request(method, params, Duration::from_secs(25))
    }
    fn fail(&mut self, e: RpcError) {
        if e.disconnected {
            self.rpc = None;
            self.login_id = None;
            self.login_started = None;
            self.state.login_url = None;
            self.state.user_code = None;
        }
        self.failures = self.failures.saturating_add(1);
        let delay = e
            .retry_after
            .unwrap_or((self.interval * 2u64.saturating_pow(self.failures.min(5))).min(600))
            .max(self.interval)
            .min(3600);
        self.state.next_retry_secs = delay;
        self.next_poll = Instant::now() + Duration::from_secs(delay);
        self.set_status(Status::Error, e.message);
    }
    fn refresh(&mut self) -> Result<(), RpcError> {
        let resetting = self.state.status == Status::Resetting;
        let a = self.request("account/read", json!({"refreshToken":false}))?;
        if a.get("account").is_none_or(|v| v.is_null()) {
            self.state.quota = None;
            self.state.email = None;
            self.state.plan = None;
            if self.login_id.is_none() {
                self.set_status(
                    Status::LoggedOut,
                    if self.backend.uses_tokens() {
                        "请点击浏览器登录，登录后自动查询额度"
                    } else {
                        "请登录 OpenAI 账号"
                    },
                );
            }
            self.next_poll = Instant::now() + Duration::from_secs(600);
            return Ok(());
        }
        if a["account"]["type"].as_str() != Some("chatgpt") {
            return Err(logical(
                "此工具监测 ChatGPT 订阅额度，请使用 OpenAI 账号登录",
            ));
        }
        self.state.email = a["account"]["email"].as_str().map(String::from);
        self.state.plan = a["account"]["planType"].as_str().map(String::from);
        let response = self.request("account/rateLimits/read", json!({}))?;
        if self.backend.uses_tokens() {
            if let Some(email) = response.get("email").and_then(serde_json::Value::as_str) {
                self.state.email = Some(email.into());
            }
            if let Some(plan) = response.get("planType").and_then(serde_json::Value::as_str) {
                self.state.plan = Some(plan.into());
            }
        }
        let quota = Quota::parse(response, now()).map_err(|s| logical(&s))?;
        if let (Some(bound), Some(current)) = (&self.bound_account, &quota.account_id) {
            if bound != current {
                return Err(logical("账号身份发生变化，已暂停更新；请退出并重新登录"));
            }
        }
        if self.bound_account.is_none() {
            self.bound_account = quota.account_id.clone();
        }
        self.state.quota = Some(quota);
        self.failures = 0;
        self.state.next_retry_secs = 0;
        self.state.pending = store::load_journal(&self.home)
            .map_err(|s| logical(&s))?
            .pending
            .is_some();
        self.next_poll = Instant::now() + Duration::from_secs(self.interval);
        if !resetting {
            self.set_status(
                Status::Ready,
                if self.state.pending {
                    "上次重置结果待核对，请点击“核对重置”"
                } else {
                    "已同步"
                },
            );
        }
        Ok(())
    }
    fn reset(&mut self) -> Result<(), RpcError> {
        if self.backend.uses_tokens() {
            return Err(logical("token 模式仅查询额度，请在官方客户端使用重置卡"));
        }
        if self.paused {
            return Err(logical("请先恢复此账号的监测"));
        }
        self.set_status(Status::Resetting, "正在核对账号与卡片");
        let mut journal = store::load_journal(&self.home).map_err(|s| logical(&s))?;
        // Always verify the current identity; reuse a durable unresolved operation verbatim.
        self.refresh()?;
        let q = self
            .state
            .quota
            .as_ref()
            .ok_or_else(|| logical("请先登录"))?;
        if q.account_id.as_ref().is_none_or(|id| id.is_empty()) {
            return Err(logical("服务端未返回账号标识，暂不使用重置卡"));
        }
        if let Some(p) = &journal.pending {
            if p.account_id != q.account_id {
                return Err(logical("上次操作所属账号与当前账号不一致，已停止重试"));
            }
        } else {
            let card = q.choose_card(now()).map_err(|s| logical(&s))?;
            journal.pending = Some(PendingReset {
                idempotency_key: uuid::Uuid::new_v4().to_string(),
                credit_id: card.id.clone(),
                account_id: q.account_id.clone(),
                created_at: now(),
            });
            // If the journal cannot be durably saved, no redemption is submitted.
            store::save_journal(&self.home, &journal).map_err(|s| logical(&s))?;
        }
        self.state.pending = true;
        self.set_status(Status::Resetting, "正在使用最早到期的重置卡");
        let p = journal.pending.as_ref().unwrap();
        let response = self.request(
            "account/rateLimitResetCredit/consume",
            json!({"idempotencyKey":p.idempotency_key,"creditId":p.credit_id}),
        )?;
        let outcome = response["outcome"].as_str().unwrap_or("");
        let text = match outcome {
            "reset" => "已使用 1 张最早到期卡，额度已重新查询",
            "alreadyRedeemed" => "已核实上次重置成功，未重复消耗卡片",
            "nothingToReset" => "没有可重置的额度，未消耗卡片",
            "noCredit" => "所选卡片已不可用，请刷新后重试",
            _ => return Err(logical("重置响应无法识别，已保留操作编号供核对")),
        };
        journal.pending = None;
        journal.last_outcome = Some(outcome.into());
        journal.last_completed_at = Some(now());
        store::save_journal(&self.home, &journal).map_err(|s| logical(&s))?;
        self.state.pending = false;
        // Only an authoritative read changes percentages and counts.
        match self.refresh() {
            Ok(()) => {
                self.set_status(Status::Ready, text);
            }
            Err(e) => {
                self.fail(e);
                self.state.message = format!("重置结果：{outcome}；额度刷新失败，稍后重试");
                self.emit();
            }
        }
        Ok(())
    }
    fn login(&mut self, device: bool) -> Result<(), RpcError> {
        if self.backend.uses_tokens() {
            self.browser_login = None;
            self.login_id = None;
            self.login_started = None;
            self.state.login_url = None;
            let login = BrowserLogin::start().map_err(|s| logical(&s))?;
            self.login_id = Some(login.id.clone());
            self.login_started = Some(Instant::now());
            self.state.login_url = Some(login.url.clone());
            self.state.user_code = None;
            let url = login.url.clone();
            self.browser_login = Some(login);
            self.set_status(
                Status::WaitingLogin,
                "请在官方网页完成登录，完成后自动导入 token 并刷新额度",
            );
            let _ = self.events.send(Event::OpenLogin(url));
            (self.wake)();
            return Ok(());
        }
        // Reauthentication must remain possible after an ambiguous redemption.
        // reset() compares the durable accountId before retrying that operation.
        store::load_journal(&self.home).map_err(|s| logical(&s))?;
        if let Some(id) = self.login_id.take() {
            let _ = self.request("account/login/cancel", json!({"loginId":id}));
        }
        let response = self.request(
            "account/login/start",
            json!({"type":if device{"chatgptDeviceCode"}else{"chatgpt"}}),
        )?;
        let raw = response[if device { "verificationUrl" } else { "authUrl" }]
            .as_str()
            .ok_or_else(|| logical("登录响应缺少官方地址"))?;
        if !crate::runtime::trusted_auth_url(raw) {
            return Err(logical("登录地址不是受信任的 OpenAI 官方地址"));
        }
        self.login_id = response["loginId"].as_str().map(String::from);
        self.login_started = Some(Instant::now());
        self.state.login_url = Some(raw.into());
        self.state.user_code = response["userCode"].as_str().map(String::from);
        self.set_status(Status::WaitingLogin, "请在浏览器中完成登录");
        let _ = self.events.send(Event::OpenLogin(raw.into()));
        (self.wake)();
        Ok(())
    }
    fn import_token(&mut self, raw: String) -> Result<(), RpcError> {
        if !self.backend.uses_tokens() {
            return Err(logical("请先在设置中清空官方组件路径，切换到 token 查询"));
        }
        token::save_token(&self.home, &raw).map_err(|s| logical(&s))?;
        self.activate_imported_token()
    }
    fn activate_imported_token(&mut self) -> Result<(), RpcError> {
        self.browser_login = None;
        self.login_id = None;
        self.login_started = None;
        self.state.login_url = None;
        self.token_client = None;
        self.bound_account = None;
        self.state.quota = None;
        self.state.email = None;
        self.state.plan = None;
        self.paused = false;
        self.set_status(Status::Syncing, "登录已保存，正在查询额度");
        self.refresh()
    }
    fn notifications(&mut self) -> Result<(), RpcError> {
        if let Some(login) = &mut self.browser_login {
            let result = login.poll(&self.home);
            if !matches!(&result, Ok(None)) {
                self.browser_login = None;
                self.login_id = None;
                self.login_started = None;
                self.state.login_url = None;
                self.state.user_code = None;
                result.map_err(|s| logical(&s))?;
                self.activate_imported_token()?;
            }
        }
        if let Some(rpc) = &mut self.rpc {
            rpc.drain()?;
        }
        let notifications = self
            .rpc
            .as_mut()
            .map(|r| std::mem::take(&mut r.notifications))
            .unwrap_or_default();
        for n in notifications {
            if n["method"] == "account/login/completed"
                && self.login_id.is_some()
                && self.login_id.as_deref() == n["params"]["loginId"].as_str()
            {
                self.login_id = None;
                self.login_started = None;
                self.state.login_url = None;
                self.state.user_code = None;
                if n["params"]["success"] == true {
                    self.bound_account = None;
                    self.state.quota = None;
                    self.refresh()?;
                } else {
                    self.set_status(Status::LoggedOut, "登录未完成，请重试");
                }
            }
            if n["method"] == "account/updated"
                && n["params"].get("authMode").is_some_and(|v| v.is_null())
            {
                self.state.quota = None;
                self.set_status(Status::LoggedOut, "登录已失效，请重新登录");
            }
            // Partial quota notifications are not treated as full snapshots: periodic reads
            // retain authoritative card counts and account identity without an event/poll loop.
        }
        Ok(())
    }
    fn run(mut self) {
        match store::load_journal(&self.home) {
            Ok(j) => self.state.pending = j.pending.is_some(),
            Err(s) => {
                self.set_status(Status::Error, s);
            }
        }
        if self.paused {
            self.state.status = Status::Paused;
            self.state.message = "已暂停监测".into();
        }
        self.emit();
        // Account-specific stagger prevents synchronized request bursts.
        let stagger = self.id.bytes().fold(0u64, |a, b| a + b as u64) % 700;
        self.next_poll = Instant::now() + Duration::from_millis(stagger);
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            let c = self
                .rx
                .recv_timeout(Duration::from_millis(if self.login_id.is_some() {
                    150
                } else {
                    500
                }));
            let result = match c {
                Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Command::Interval(i)) => {
                    self.interval = if i == 10 { 10 } else { 30 };
                    self.next_poll = Instant::now();
                    Ok(())
                }
                Ok(Command::Pause(p)) => {
                    self.paused = p;
                    if p {
                        self.set_status(Status::Paused, "已暂停监测");
                        self.rpc = None;
                    } else {
                        self.next_poll = Instant::now();
                    }
                    Ok(())
                }
                Ok(Command::Refresh) if !self.paused => self.refresh(),
                Ok(Command::Reset) => self.reset(),
                Ok(Command::Login { device }) => {
                    self.paused = false;
                    self.login(device)
                }
                Ok(Command::UseToken(raw)) => self.import_token(raw),
                Ok(Command::SwitchCodex(path)) => {
                    self.switch_codex(path);
                    Ok(())
                }
                Ok(Command::SetSharedCodex(path)) => {
                    self.shared_auth = path.clone();
                    if let Some(client) = &mut self.token_client {
                        client.set_shared_auth(path);
                    }
                    if self.shared_auth.is_some() {
                        self.next_poll = Instant::now();
                    }
                    Ok(())
                }
                Ok(Command::CancelLogin) => {
                    self.browser_login = None;
                    if let Some(id) = self.login_id.take() {
                        if !self.backend.uses_tokens() {
                            let _ = self.request("account/login/cancel", json!({"loginId":id}));
                        }
                    }
                    self.state.login_url = None;
                    self.state.user_code = None;
                    self.login_started = None;
                    self.set_status(Status::LoggedOut, "登录已取消");
                    Ok(())
                }
                Ok(Command::Logout) => {
                    if self.state.pending {
                        Err(logical("上次重置结果尚未核对，请先核对后退出"))
                    } else {
                        match self.request("account/logout", json!({})) {
                            Ok(_) => {
                                self.state.quota = None;
                                self.bound_account = None;
                                self.state.email = None;
                                self.state.plan = None;
                                self.rpc = None;
                                self.paused = true;
                                self.set_status(Status::LoggedOut, "已退出登录");
                                let _ = self.events.send(Event::Removed(self.id.clone()));
                                (self.wake)();
                                Ok(())
                            }
                            Err(e) => Err(e),
                        }
                    }
                }
                _ => Ok(()),
            };
            if let Err(e) = result {
                self.fail(e);
            }
            if !self.paused {
                if let Err(e) = self.notifications() {
                    self.fail(e);
                }
                if self
                    .login_started
                    .is_some_and(|t| t.elapsed() > Duration::from_secs(600))
                {
                    if let Some(id) = self.login_id.take() {
                        if !self.backend.uses_tokens() {
                            let _ = self.request("account/login/cancel", json!({"loginId":id}));
                        }
                    }
                    self.browser_login = None;
                    self.login_started = None;
                    self.state.login_url = None;
                    self.state.user_code = None;
                    self.set_status(Status::LoggedOut, "登录已超时，请重试");
                }
                if self.login_id.is_none() && Instant::now() >= self.next_poll {
                    if let Err(e) = self.refresh() {
                        self.fail(e);
                    }
                }
            }
        }
        self.rpc = None;
    }
}
fn logical(message: &str) -> RpcError {
    RpcError {
        message: message.into(),
        retry_after: None,
        disconnected: false,
    }
}

pub fn demo_state(id: &str, index: usize) -> ViewState {
    let used = [24, 78, 100][index % 3];
    let time = now();
    let mut q=Quota::parse(json!({"accountId":format!("demo-{index}"),"rateLimits":{"limitId":"codex","primary":{"usedPercent":used,"windowDurationMins":10080,"resetsAt":time+129600}},"rateLimitResetCredits":{"availableCount":3,"credits":[
        {"id":"long","resetType":"codexRateLimits","status":"available","grantedAt":time-86400,"expiresAt":time+864000},
        {"id":"soon","resetType":"codexRateLimits","status":"available","grantedAt":time-3600,"expiresAt":time+7200},
        {"id":"forever","resetType":"codexRateLimits","status":"available","grantedAt":time-600,"expiresAt":null}]}}),time).unwrap();
    q.rate_limits.credits = Some(crate::model::CreditBalance {
        balance: Some(if index % 3 == 0 { "200.00" } else { "0" }.into()),
        has_credits: index % 3 == 0,
        unlimited: false,
    });
    if index % 3 == 1 {
        q.rate_limit_reset_credits = Some(crate::model::ResetCards {
            available_count: 0,
            credits: Some(vec![]),
        });
    }
    let mut s = ViewState::new(id.into(), Some(q));
    s.email = Some(format!("demo{}@example.com", index + 1));
    s.plan = Some("pro".into());
    s.status = Status::Ready;
    s.message = "演示数据 · 未连接真实账号".into();
    s
}
