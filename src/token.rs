//! Read existing credentials and query usage without starting the Codex agent.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const MAX_JSON_BYTES: u64 = 1024 * 1024;
const SOURCE_FILE: &str = "token-source.json";
const SECRET_FILE: &str = "token.secret";
const AUTH_FILE: &str = "auth.json";
static SHARED_ACCOUNTS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, PathBuf>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

// Deliberately no Debug implementation: credentials must never appear in logs.
#[derive(Clone, Serialize, Deserialize)]
struct Credentials {
    access_token: String,
    account_id: Option<String>,
    email: Option<String>,
    plan: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: Option<i64>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    last_refresh: Option<String>,
    #[serde(default)]
    source_auth: Option<Value>,
}

fn read_limited(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|_| "无法读取 token 文件，请检查路径与权限")?;
    let mut bytes = Vec::new();
    file.take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取 token 文件")?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err("登录凭据文件过大，请重新网页登录".into());
    }
    Ok(bytes)
}

pub(crate) fn claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

fn text(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|k| {
        value
            .get(k)?
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(String::from)
    })
}

fn parse_credentials(raw: &str) -> Result<Credentials, String> {
    let raw = raw.trim().trim_start_matches('\u{feff}');
    let value: Value = if raw.starts_with('{') {
        serde_json::from_str(raw).map_err(|_| "token JSON 格式无效")?
    } else {
        json!({"access_token": raw.strip_prefix("Bearer ").unwrap_or(raw).trim()})
    };
    let tokens = value
        .get("tokens")
        .filter(|v| v.is_object())
        .unwrap_or(&value);
    let access_token = text(tokens, &["access_token", "accessToken"])
        .ok_or("未找到 access_token；API Key 不能用于查询订阅额度")?;
    if access_token.len() > 64 * 1024
        || access_token.chars().any(char::is_whitespace)
        || reqwest::header::HeaderValue::from_str(&access_token).is_err()
    {
        return Err("access token 格式无效".into());
    }
    // Claims are display/routing hints only. The service validates the token.
    let access = claims(&access_token);
    let identity = claims(&text(tokens, &["id_token", "idToken"]).unwrap_or_default());
    let auth = access
        .get("https://api.openai.com/auth")
        .unwrap_or(&Value::Null);
    let identity_auth = identity
        .get("https://api.openai.com/auth")
        .unwrap_or(&Value::Null);
    let profile = access
        .get("https://api.openai.com/profile")
        .unwrap_or(&Value::Null);
    let account_id = text(tokens, &["account_id", "accountId", "chatgpt_account_id"])
        .or_else(|| text(auth, &["chatgpt_account_id"]))
        .or_else(|| text(identity_auth, &["chatgpt_account_id"]));
    if account_id
        .as_ref()
        .is_some_and(|s| reqwest::header::HeaderValue::from_str(s).is_err())
    {
        return Err("账号标识格式无效".into());
    }
    Ok(Credentials {
        access_token,
        account_id,
        email: text(&identity, &["email"])
            .or_else(|| text(profile, &["email"]))
            .or_else(|| text(&value, &["email"])),
        plan: text(auth, &["chatgpt_plan_type"])
            .or_else(|| text(identity_auth, &["chatgpt_plan_type"]))
            .or_else(|| text(&value, &["plan_type"])),
        refresh_token: text(tokens, &["refresh_token", "refreshToken"]),
        expires_at: access.get("exp").and_then(Value::as_i64).or_else(|| {
            value
                .get("expires_in")
                .and_then(Value::as_i64)
                .map(|s| crate::model::now().saturating_add(s))
        }),
        id_token: text(tokens, &["id_token", "idToken"]),
        last_refresh: text(&value, &["last_refresh"]),
        source_auth: value
            .get("tokens")
            .filter(|v| v.is_object())
            .map(|_| value.clone()),
    })
}

fn auth_document(credentials: &Credentials) -> Value {
    let mut document = credentials
        .source_auth
        .clone()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    document["auth_mode"] = json!("chatgpt");
    document["OPENAI_API_KEY"] = Value::Null;
    let mut tokens = document
        .get("tokens")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    tokens["id_token"] = json!(credentials.id_token.as_deref().unwrap_or(""));
    tokens["access_token"] = json!(credentials.access_token);
    tokens["refresh_token"] = json!(credentials.refresh_token.as_deref().unwrap_or(""));
    tokens["account_id"] = json!(credentials.account_id);
    document["tokens"] = tokens;
    document["last_refresh"] = json!(credentials.last_refresh.clone().unwrap_or_else(|| {
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }));
    document
}

fn persist(home: &Path, credentials: &Credentials) -> Result<(), String> {
    crate::store::atomic_json(&home.join(AUTH_FILE), &auth_document(credentials))?;
    // Complete the new file before removing a superseded encrypted copy.
    remove_if_exists(&home.join(SOURCE_FILE))?;
    remove_if_exists(&home.join(SECRET_FILE))
}

/// Persist only credentials explicitly imported or obtained by this app's login.
pub fn save_token(home: &Path, raw: &str) -> Result<(), String> {
    let credentials = parse_credentials(raw)?;
    persist(home, &credentials)
}

pub fn forget(home: &Path) -> Result<(), String> {
    remove_if_exists(&home.join(AUTH_FILE))?;
    remove_if_exists(&home.join(SOURCE_FILE))?;
    remove_if_exists(&home.join(SECRET_FILE))
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("无法移除本程序保存的 token 引用".into()),
    }
}

fn load(home: &Path) -> Result<Option<Credentials>, String> {
    let auth = home.join(AUTH_FILE);
    if auth.is_file() {
        let bytes = read_limited(&auth)?;
        return parse_credentials(
            std::str::from_utf8(&bytes).map_err(|_| "本工具的 auth.json 格式无效")?,
        )
        .map(Some);
    }
    let secret = home.join(SECRET_FILE);
    if secret.is_file() {
        let bytes: Vec<u8> = serde_json::from_slice(&read_limited(&secret)?)
            .map_err(|_| "已保存的 token 损坏，请重新导入")?;
        let credentials: Credentials =
            serde_json::from_slice(&unprotect(&bytes)?).map_err(|_| "已保存的 token 格式无效")?;
        persist(home, &credentials)?;
        return Ok(Some(credentials));
    }
    if let Some(raw) = crate::legacy::load(home)? {
        let credentials = parse_credentials(&raw)?;
        // Commit the replacement before any later cleanup can remove old data.
        save_token(home, &raw)?;
        return Ok(Some(credentials));
    }
    Ok(None)
}

pub(crate) fn prepare_relocation(home: &Path) -> Result<(), String> {
    // Decode old stores while their original path-derived key is still valid.
    // Keep opaque credentials and their key index when a key is unavailable.
    let _ = load(home);
    crate::legacy::remember_home(home)
}

pub fn codex_auth_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("CODEX_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".codex")))
        .ok_or("无法确定 Codex 数据目录")?;
    if !home.is_absolute() {
        return Err("CODEX_HOME 必须是完整目录路径".into());
    }
    Ok(home.join(AUTH_FILE))
}

fn write_codex_auth(home: &Path, destination: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut selected = SHARED_ACCOUNTS
        .lock()
        .map_err(|_| "Codex 凭据正在写入，请稍后重试")?;
    if !destination.is_absolute()
        || destination.file_name().and_then(|n| n.to_str()) != Some(AUTH_FILE)
    {
        return Err("Codex 凭据目标路径无效".into());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "账号的 auth.json 无效")?;
    parse_credentials(&value.to_string())?;
    // Only an explicit switch reaches this replacement. Later synchronization
    // is restricted to the selected monitor account and destination.
    if destination.exists() {
        let original = read_limited(destination)?;
        let backup = destination.with_file_name("auth.json.quotamonitor-backup");
        crate::store::atomic_bytes(&backup, &original)
            .map_err(|_| "无法备份 Codex 原凭据，已停止切换")?;
    }
    crate::store::atomic_bytes(destination, bytes)
        .map_err(|_| "无法写入 Codex auth.json，原凭据备份已保留")?;
    selected.insert(destination.to_path_buf(), home.to_path_buf());
    Ok(())
}

#[cfg(windows)]
fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let success = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err("无法访问此 Windows 用户加密保存的 token，请重新导入".into());
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}
#[cfg(all(windows, test))]
fn protect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    crypt(bytes, true)
}
#[cfg(windows)]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    crypt(bytes, false)
}
#[cfg(all(not(windows), test))]
fn protect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    Ok(bytes.to_vec())
}
#[cfg(not(windows))]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    Ok(bytes.to_vec())
}

pub struct TokenClient {
    home: PathBuf,
    client: Client,
    credentials: Option<Credentials>,
    tried_id_upgrade: bool,
    shared_auth: Option<PathBuf>,
}
pub(crate) fn http_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("QuotaMonitor/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法初始化额度查询连接".into())
}
impl TokenClient {
    pub fn new(home: &Path) -> Result<Self, String> {
        let client = http_client()?;
        Ok(Self {
            home: home.to_path_buf(),
            client,
            credentials: None,
            tried_id_upgrade: false,
            shared_auth: None,
        })
    }

    pub fn account(&mut self) -> Result<Value, String> {
        self.credentials = load(&self.home)?;
        self.sync_shared();
        if !self.tried_id_upgrade
            && self
                .credentials
                .as_ref()
                .is_some_and(|c| c.id_token.is_none() && c.refresh_token.is_some())
        {
            self.tried_id_upgrade = true;
            // Older versions discarded id_token. Reuse their refresh token to
            // recover it, while keeping a working quota token on network failure.
            let _ = self.renew();
        }
        Ok(match &self.credentials {
            Some(c) => json!({"account":{"type":"chatgpt", "email":c.email, "planType":c.plan}}),
            None => json!({"account":null}),
        })
    }

    pub fn set_shared_auth(&mut self, path: Option<PathBuf>) {
        if let Ok(mut selected) = SHARED_ACCOUNTS.lock() {
            if let Some(path) = &path {
                selected
                    .entry(path.clone())
                    .or_insert_with(|| self.home.clone());
            } else if let Some(old) = &self.shared_auth {
                if selected.get(old) == Some(&self.home) {
                    selected.remove(old);
                }
            }
        }
        self.shared_auth = path;
    }

    fn sync_shared(&mut self) {
        // This path is set only after the user explicitly switches this account
        // to Codex. No global auth lookup happens for an unselected account.
        let Some(path) = self.shared_auth.clone() else {
            return;
        };
        // Keep selection and file writes under the same lock. An older worker
        // must not replace a newer selection, even for duplicate account rows.
        let Ok(selected) = SHARED_ACCOUNTS.lock() else {
            return;
        };
        if selected.get(&path) != Some(&self.home) {
            return;
        }
        let Some(current) = self.credentials.as_ref() else {
            return;
        };
        let Ok(bytes) = read_limited(&path) else {
            return;
        };
        let Ok(raw) = std::str::from_utf8(&bytes) else {
            return;
        };
        let Ok(shared) = parse_credentials(raw) else {
            return;
        };
        if current.account_id.is_none() || current.account_id != shared.account_id {
            return;
        }
        let timestamp = |c: &Credentials| {
            c.last_refresh
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp_millis())
                .unwrap_or(0)
        };
        match timestamp(current).cmp(&timestamp(&shared)) {
            std::cmp::Ordering::Less => {
                if persist(&self.home, &shared).is_ok() {
                    self.credentials = Some(shared);
                }
            }
            std::cmp::Ordering::Greater => {
                let _ = crate::store::atomic_json(&path, &auth_document(current));
            }
            std::cmp::Ordering::Equal => {}
        }
    }

    pub fn quota(&mut self) -> Result<Value, crate::rpc::RpcError> {
        self.quota_at(USAGE_URL)
    }

    fn quota_at(&mut self, endpoint: &str) -> Result<Value, crate::rpc::RpcError> {
        self.quota_attempt(endpoint, true)
    }

    fn renew(&mut self) -> Result<(), String> {
        let current = self.credentials.clone().ok_or("请先完成网页登录")?;
        let refresh = current
            .refresh_token
            .as_deref()
            .ok_or("登录已失效，请重新网页登录")?;
        let response = match crate::login::refresh(&self.client, refresh) {
            Ok(response) => response,
            Err(message) => {
                self.sync_shared();
                if self
                    .credentials
                    .as_ref()
                    .is_some_and(|c| c.access_token != current.access_token)
                {
                    return Ok(());
                }
                return Err(message);
            }
        };
        self.apply_refresh_response(response)
    }

    fn apply_refresh_response(&mut self, response: Value) -> Result<(), String> {
        let current = self.credentials.clone().ok_or("请先完成网页登录")?;
        let mut updated = parse_credentials(&response.to_string())?;
        if let (Some(old), Some(new)) = (&current.account_id, &updated.account_id) {
            if old != new {
                return Err("续期后的账号不一致，请重新网页登录".into());
            }
        }
        updated.account_id = updated.account_id.or(current.account_id);
        updated.email = updated.email.or(current.email);
        updated.plan = updated.plan.or(current.plan);
        updated.refresh_token = updated.refresh_token.or(current.refresh_token);
        updated.id_token = updated.id_token.or(current.id_token);
        updated.source_auth = current.source_auth;
        updated.last_refresh =
            Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        persist(&self.home, &updated)?;
        self.credentials = Some(updated);
        self.sync_shared();
        Ok(())
    }

    pub fn replace_codex_auth(&mut self, destination: &Path) -> Result<(), String> {
        self.account()?;
        if self.credentials.as_ref().is_some_and(|c| {
            c.id_token.is_none() || c.expires_at.is_some_and(|t| t <= crate::model::now() + 30)
        }) {
            self.renew()?;
        }
        let credentials = self
            .credentials
            .as_ref()
            .ok_or("请先完成此账号的网页登录")?;
        if credentials.id_token.as_deref().is_none_or(str::is_empty)
            || credentials
                .id_token
                .as_ref()
                .is_none_or(|s| s.split('.').count() != 3 || !claims(s).is_object())
            || credentials
                .refresh_token
                .as_deref()
                .is_none_or(str::is_empty)
            || credentials.account_id.as_deref().is_none_or(str::is_empty)
        {
            return Err("此账号缺少完整登录信息，请重新网页登录后切换".into());
        }
        let bytes = read_limited(&self.home.join(AUTH_FILE))?;
        write_codex_auth(&self.home, destination, &bytes)?;
        self.shared_auth = Some(destination.to_path_buf());
        Ok(())
    }

    fn quota_attempt(
        &mut self,
        endpoint: &str,
        allow_refresh: bool,
    ) -> Result<Value, crate::rpc::RpcError> {
        let error = |message: &str, retry_after| crate::rpc::RpcError {
            message: message.into(),
            retry_after,
            disconnected: false,
        };
        if allow_refresh
            && self.credentials.as_ref().is_some_and(|c| {
                c.refresh_token.is_some()
                    && c.expires_at.is_some_and(|t| t <= crate::model::now() + 30)
            })
        {
            self.renew().map_err(|s| error(&s, None))?;
            return self.quota_attempt(endpoint, false);
        }
        let c = self
            .credentials
            .as_ref()
            .ok_or_else(|| error("请先完成网页登录", None))?;
        let mut request = self
            .client
            .get(endpoint)
            .bearer_auth(&c.access_token)
            .header("Accept", "application/json");
        if let Some(id) = &c.account_id {
            request = request.header("ChatGPT-Account-Id", id);
        }
        let response = request
            .send()
            .map_err(|_| error("额度查询连接失败，请检查网络或本机代理", None))?;
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|s| s.to_str().ok())
            .and_then(|s| s.parse().ok());
        match response.status().as_u16() {
            200 => {}
            401 => {
                if allow_refresh && c.refresh_token.is_some() {
                    self.renew().map_err(|s| error(&s, None))?;
                    return self.quota_attempt(endpoint, false);
                }
                return Err(error("登录已过期或失效，请重新网页登录", None));
            }
            403 => return Err(error("此账号无权查询额度，请重新网页登录", None)),
            429 => {
                return Err(error(
                    "请求过于频繁，已延后重试",
                    Some(retry_after.unwrap_or(60)),
                ));
            }
            status => {
                return Err(error(
                    &format!("额度查询失败（HTTP {status}），请稍后重试"),
                    retry_after,
                ));
            }
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_JSON_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error("额度响应读取失败", None))?;
        if bytes.len() as u64 > MAX_JSON_BYTES {
            return Err(error("额度响应过大，已停止读取", None));
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| error("额度接口返回了无效数据", None))?;
        if let (Some(expected), Some(actual)) = (
            &c.account_id,
            value.get("account_id").and_then(Value::as_str),
        ) {
            if expected != actual {
                return Err(error("额度响应账号不匹配，已停止更新", None));
            }
        }
        normalize_usage(value, crate::model::now()).map_err(|s| error(&s, None))
    }
}

fn window(value: &Value, time: i64) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    json!({
        "usedPercent":value.get("used_percent"),
        "windowDurationMins":value.get("limit_window_seconds").and_then(Value::as_i64).map(|s| s / 60),
        "resetsAt":value.get("reset_at").and_then(Value::as_i64).or_else(|| value.get("reset_after_seconds").and_then(Value::as_i64).map(|s| time.saturating_add(s)))
    })
}

fn bucket(value: &Value, id: &str, name: Option<&str>, time: i64) -> Value {
    json!({"limitId":id,"limitName":name,"primary":window(&value["primary_window"],time),"secondary":window(&value["secondary_window"],time)})
}

pub fn normalize_usage(value: Value, time: i64) -> Result<Value, String> {
    let rate = value
        .get("rate_limit")
        .filter(|v| v.is_object())
        .ok_or("额度响应不完整，请稍后重试")?;
    let mut primary = bucket(rate, "codex", None, time);
    // The usage endpoint returns an object when a limit has been reached;
    // older responses used a plain string. Keep the normalized model stable.
    let reached = &value["rate_limit_reached_type"];
    primary["rateLimitReachedType"] = reached
        .as_str()
        .or_else(|| reached.get("type").and_then(Value::as_str))
        .map(|kind| json!(kind))
        .unwrap_or(Value::Null);
    if let Some(credits) = value.get("credits").filter(|v| v.is_object()) {
        primary["credits"] = json!({"balance":credits.get("balance").map(|v| if v.is_number() {Value::String(v.to_string())} else {v.clone()}),"hasCredits":credits.get("has_credits").and_then(Value::as_bool).unwrap_or(false),"unlimited":credits.get("unlimited").and_then(Value::as_bool).unwrap_or(false)});
    }
    primary["spendControlReached"] = value
        .pointer("/spend_control/reached")
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(limit) = value
        .pointer("/spend_control/individual_limit")
        .filter(|v| v.is_object())
    {
        primary["individualLimit"] = json!({"remainingPercent":limit.get("remaining_percent"),"resetsAt":limit.get("resets_at"),"limit":limit.get("limit"),"used":limit.get("used")});
    }
    let mut buckets = serde_json::Map::new();
    buckets.insert("codex".into(), primary.clone());
    if let Some(additional) = value
        .get("additional_rate_limits")
        .and_then(Value::as_array)
    {
        for row in additional {
            if let Some(id) = row
                .get("metered_feature")
                .or_else(|| row.get("limit_id"))
                .and_then(Value::as_str)
                .filter(|id| *id != "codex")
            {
                if let Some(rate) = row.get("rate_limit").filter(|r| r.is_object()) {
                    buckets.insert(
                        id.into(),
                        bucket(
                            rate,
                            id,
                            row.get("limit_name").and_then(Value::as_str),
                            time,
                        ),
                    );
                }
            }
        }
    }
    let resets = value
        .get("rate_limit_reset_credits")
        .filter(|v| v.is_object())
        .and_then(|r| {
            r.get("available_count")
                .and_then(Value::as_i64)
                .map(|count| json!({"availableCount":count,"credits":null}))
        });
    Ok(
        json!({"rateLimits":primary,"rateLimitsByLimitId":buckets,"accountId":value.get("account_id"),"ordinaryUsageAllowed":rate.get("allowed"),"rateLimitResetCredits":resets,"email":value.get("email"),"planType":value.get("plan_type")}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Quota;
    #[test]
    fn parses_usage_windows_credits_and_additional_buckets() {
        let value = json!({"account_id":"acct-a","rate_limit":{"allowed":true,"primary_window":{"used_percent":25,"limit_window_seconds":18000,"reset_at":500},"secondary_window":null},"credits":{"has_credits":true,"unlimited":false,"balance":"60324.1"},"additional_rate_limits":[{"metered_feature":"codex_mini","limit_name":"Mini","rate_limit":{"primary_window":{"used_percent":99,"limit_window_seconds":604800,"reset_after_seconds":300}}}],"rate_limit_reset_credits":{"available_count":2}});
        let q = Quota::parse(normalize_usage(value, 100).unwrap(), 100).unwrap();
        assert_eq!(q.account_id.as_deref(), Some("acct-a"));
        assert_eq!(q.rate_limits.primary.as_ref().unwrap().remaining(), 75.0);
        assert_eq!(
            q.rate_limits.primary.as_ref().unwrap().window_duration_mins,
            Some(300)
        );
        assert_eq!(
            q.rate_limits
                .credits
                .as_ref()
                .unwrap()
                .rounded_balance()
                .as_deref(),
            Some("60325")
        );
        assert_eq!(q.windows().len(), 2);
        assert_eq!(q.tightest().unwrap().1.resets_at, Some(400));
        assert_eq!(q.card_count(), Some(2));
        assert!(!q.expiry_details_known);
    }
    #[test]
    fn missing_usage_is_not_reported_as_zero() {
        assert!(normalize_usage(json!({}), 1).is_err());
        assert!(
            Quota::parse(
                normalize_usage(json!({"rate_limit":{"primary_window":{}}}), 1).unwrap(),
                1
            )
            .is_err()
        );
        let q = Quota::parse(normalize_usage(json!({"rate_limit":{}}), 1).unwrap(), 1).unwrap();
        assert!(q.tightest().is_none());
        assert_eq!(q.card_count(), None);
    }
    #[test]
    fn exhausted_account_accepts_object_or_string_limit_markers() {
        for marker in [
            json!({"type":"rate_limit_reached","details":"default"}),
            json!("rate_limit_reached"),
            Value::Null,
            json!({"future_detail":true}),
        ] {
            let value = json!({"rate_limit":{"allowed":false,"limit_reached":true,"primary_window":{"used_percent":100,"limit_window_seconds":604800,"reset_at":1793382093},"secondary_window":null},"rate_limit_reached_type":marker,"credits":{"has_credits":true,"unlimited":false,"balance":"62083.9556045000"},"spend_control":{"reached":false,"individual_limit":null},"rate_limit_reset_credits":{"available_count":0}});
            let q = Quota::parse(normalize_usage(value, 100).unwrap(), 100).unwrap();
            assert_eq!(q.tightest().unwrap().1.remaining(), 0.0);
            assert_eq!(q.tightest().unwrap().1.window_duration_mins, Some(10080));
            assert_eq!(q.ordinary_usage_allowed, Some(false));
            assert!(q.depleted());
            assert_eq!(q.card_count(), Some(0));
            assert_eq!(
                q.rate_limits
                    .credits
                    .as_ref()
                    .unwrap()
                    .rounded_balance()
                    .as_deref(),
                Some("62084")
            );
        }
    }
    #[test]
    fn global_auth_file_references_are_not_imported() {
        let root = std::env::temp_dir().join(format!("quota-token-{}", uuid::Uuid::new_v4()));
        let home = root.join("monitor");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&home).unwrap();
        let auth = root.join("auth.json");
        fs::write(
            &auth,
            r#"{"tokens":{"access_token":"first","account_id":"a"}}"#,
        )
        .unwrap();
        fs::write(home.join(SOURCE_FILE), json!({"path":auth}).to_string()).unwrap();
        assert!(load(&home).unwrap().is_none());
        save_token(&home, "explicitly-imported").unwrap();
        assert_eq!(
            load(&home).unwrap().unwrap().access_token,
            "explicitly-imported"
        );
        forget(&home).unwrap();
        assert!(auth.is_file());
        assert!(load(&home).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pasted_credentials_round_trip_and_never_enter_settings() {
        let root = std::env::temp_dir().join(format!("quota-secret-{}", uuid::Uuid::new_v4()));
        save_token(&root, "Bearer test-access-token").unwrap();
        assert_eq!(
            load(&root).unwrap().unwrap().access_token,
            "test-access-token"
        );
        assert!(!root.join("settings.json").exists());
        let saved: Value =
            serde_json::from_slice(&fs::read(root.join(AUTH_FILE)).unwrap()).unwrap();
        assert_eq!(saved["auth_mode"], "chatgpt");
        assert!(saved["OPENAI_API_KEY"].is_null());
        assert_eq!(saved["tokens"]["access_token"], "test-access-token");
        assert!(saved["last_refresh"].is_string());
        assert!(!root.join(SECRET_FILE).exists());
        assert!(parse_credentials(r#"{"OPENAI_API_KEY":"sk-test"}"#).is_err());
        assert!(parse_credentials("bad\r\ntoken").is_err());
        forget(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn fixture_auth() -> Value {
        let claims = json!({"email":"fixture@example.com","https://api.openai.com/auth":{"chatgpt_account_id":"fixture-account","chatgpt_plan_type":"pro"}});
        let id = format!(
            "test.{}.test",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        );
        json!({"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":id,"access_token":"fixture-access","refresh_token":"fixture-refresh","account_id":"fixture-account"},"last_refresh":"2026-10-25T00:00:00Z"})
    }

    #[test]
    fn codex_auth_round_trip_and_replacement_preserve_every_field() {
        let root = std::env::temp_dir().join(format!("quota-auth-export-{}", uuid::Uuid::new_v4()));
        let home = root.join("monitor");
        let destination = root.join("codex/auth.json");
        let mut auth = fixture_auth();
        auth["future_field"] = json!({"keep":"unchanged"});
        save_token(&home, &auth.to_string()).unwrap();
        let saved = fs::read(home.join(AUTH_FILE)).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&saved).unwrap(), auth);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        let original = b"original credential bytes";
        fs::write(&destination, original).unwrap();
        let mut client = TokenClient::new(&home).unwrap();
        client.replace_codex_auth(&destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), saved);
        assert_eq!(
            fs::read(destination.with_file_name("auth.json.quotamonitor-backup")).unwrap(),
            original
        );
        auth["tokens"]["access_token"] = json!("second-access");
        save_token(&home, &auth.to_string()).unwrap();
        client.replace_codex_auth(&destination).unwrap();
        assert_eq!(
            fs::read(destination.with_file_name("auth.json.quotamonitor-backup")).unwrap(),
            saved
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&destination).unwrap()).unwrap(),
            auth
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn encrypted_format_migrates_without_losing_tokens() {
        let root = std::env::temp_dir().join(format!("quota-old-format-{}", uuid::Uuid::new_v4()));
        let auth = fixture_auth();
        let credentials = parse_credentials(&auth.to_string()).unwrap();
        let encrypted = protect(&serde_json::to_vec(&credentials).unwrap()).unwrap();
        crate::store::atomic_json(&root.join(SECRET_FILE), &encrypted).unwrap();
        assert_eq!(load(&root).unwrap().unwrap().id_token, credentials.id_token);
        assert!(!root.join(SECRET_FILE).exists());
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(root.join(AUTH_FILE)).unwrap()).unwrap(),
            auth
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replacement_failure_keeps_the_original_login() {
        let root =
            std::env::temp_dir().join(format!("quota-auth-failure-{}", uuid::Uuid::new_v4()));
        let home = root.join("monitor");
        let destination = root.join("codex/auth.json");
        save_token(&home, &fixture_auth().to_string()).unwrap();
        fs::create_dir_all(destination.with_file_name("auth.json.quotamonitor-backup")).unwrap();
        fs::write(&destination, b"original").unwrap();
        let mut client = TokenClient::new(&home).unwrap();
        assert!(client.replace_codex_auth(&destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        save_token(&home, "access-only").unwrap();
        assert!(client.replace_codex_auth(&destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_preserves_id_token_and_rejects_another_account() {
        let root =
            std::env::temp_dir().join(format!("quota-auth-refresh-{}", uuid::Uuid::new_v4()));
        let auth = fixture_auth();
        save_token(&root, &auth.to_string()).unwrap();
        let mut client = TokenClient::new(&root).unwrap();
        client.account().unwrap();
        client
            .apply_refresh_response(
                json!({"access_token":"new-access","refresh_token":"new-refresh"}),
            )
            .unwrap();
        let saved = fs::read(root.join(AUTH_FILE)).unwrap();
        let value: Value = serde_json::from_slice(&saved).unwrap();
        assert_eq!(value["tokens"]["id_token"], auth["tokens"]["id_token"]);
        assert_eq!(value["tokens"]["refresh_token"], "new-refresh");
        assert_eq!(value["tokens"]["account_id"], "fixture-account");
        assert!(
            client
                .apply_refresh_response(
                    json!({"access_token":"wrong-access","account_id":"other-account"})
                )
                .is_err()
        );
        assert_eq!(fs::read(root.join(AUTH_FILE)).unwrap(), saved);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicitly_selected_codex_file_shares_refreshes_without_overwriting_other_accounts() {
        let root = std::env::temp_dir().join(format!("quota-auth-shared-{}", uuid::Uuid::new_v4()));
        let home = root.join("monitor");
        let destination = root.join("codex/auth.json");
        let mut original = fixture_auth();
        original["last_refresh"] = json!("2020-01-01T00:00:00Z");
        save_token(&home, &original.to_string()).unwrap();
        let mut client = TokenClient::new(&home).unwrap();
        client.replace_codex_auth(&destination).unwrap();
        let mut codex = original.clone();
        codex["tokens"]["access_token"] = json!("codex-updated-access");
        codex["tokens"]["refresh_token"] = json!("codex-updated-refresh");
        codex["last_refresh"] = json!("2020-01-01T00:00:01Z");
        crate::store::atomic_json(&destination, &codex).unwrap();
        client.account().unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(home.join(AUTH_FILE)).unwrap()).unwrap(),
            codex
        );
        client.apply_refresh_response(json!({"access_token":"monitor-updated-access","refresh_token":"monitor-updated-refresh"})).unwrap();
        let local = fs::read(home.join(AUTH_FILE)).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), local);
        codex["tokens"]["account_id"] = json!("another-account");
        codex["last_refresh"] = json!("2099-01-01T00:00:00Z");
        crate::store::atomic_json(&destination, &codex).unwrap();
        let foreign = fs::read(&destination).unwrap();
        client.account().unwrap();
        assert_eq!(fs::read(home.join(AUTH_FILE)).unwrap(), local);
        client
            .apply_refresh_response(json!({"access_token":"monitor-next-access"}))
            .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), foreign);
        // A previous worker cannot overwrite a newly selected row for the
        // same ChatGPT account, including an in-flight refresh completion.
        let other_home = root.join("second-monitor");
        save_token(&other_home, &original.to_string()).unwrap();
        let mut other = TokenClient::new(&other_home).unwrap();
        other.replace_codex_auth(&destination).unwrap();
        let selected = fs::read(&destination).unwrap();
        client
            .apply_refresh_response(json!({"access_token":"previous-worker-access"}))
            .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), selected);
        client.set_shared_auth(Some(destination.clone()));
        client.account().unwrap();
        assert_eq!(fs::read(&destination).unwrap(), selected);
        client.set_shared_auth(None);
        other.set_shared_auth(None);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn http_request_routes_token_and_handles_auth_errors() {
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
            thread,
        };
        let root = std::env::temp_dir().join(format!("quota-http-{}", uuid::Uuid::new_v4()));
        save_token(
            &root,
            r#"{"access_token":"test-only-token","account_id":"acct-a"}"#,
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/usage", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for status in [200, 401, 429] {
                let (mut socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line.to_lowercase());
                }
                assert!(headers.contains("authorization: bearer test-only-token"));
                assert!(headers.contains("chatgpt-account-id: acct-a"));
                let body = r#"{"account_id":"acct-a","rate_limit":{"primary_window":{"used_percent":20}}}"#;
                write!(socket,"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\nRetry-After: 91\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let mut client = TokenClient::new(&root).unwrap();
        client.client = Client::builder().no_proxy().build().unwrap();
        client.account().unwrap();
        let q = Quota::parse(client.quota_at(&endpoint).unwrap(), 1).unwrap();
        assert_eq!(q.tightest().unwrap().1.remaining(), 80.0);
        assert!(
            client
                .quota_at(&endpoint)
                .unwrap_err()
                .message
                .contains("登录已过期")
        );
        assert_eq!(
            client.quota_at(&endpoint).unwrap_err().retry_after,
            Some(91)
        );
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
