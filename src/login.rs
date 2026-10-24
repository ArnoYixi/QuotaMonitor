//! Browser OAuth with PKCE. Only a loopback listener and HTTPS requests are used.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::blocking::Client;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    time::{Duration, Instant},
};

// Public native-client identifier used by the open-source Codex login flow.
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";

pub struct BrowserLogin {
    listener: TcpListener,
    client: Client,
    pub id: String,
    pub url: String,
    state: String,
    verifier: String,
    nonce: String,
    redirect: String,
    started: Instant,
}

fn random_string() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl BrowserLogin {
    pub fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:1455")
            .map_err(|_| "登录回调端口 1455 被占用，请先结束其他正在进行的 Codex 登录，再重试")?;
        Self::with_listener(listener, REDIRECT_URI.into())
    }

    fn with_listener(listener: TcpListener, redirect: String) -> Result<Self, String> {
        listener
            .set_nonblocking(true)
            .map_err(|_| "无法启动登录回调")?;
        let verifier = random_string();
        let state = random_string();
        let nonce = random_string();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = url::Url::parse("https://auth.openai.com/oauth/authorize").unwrap();
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", redirect.as_str()),
            ("scope", "openid profile email offline_access"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
            ("originator", "quotamonitor"),
        ]);
        Ok(Self {
            listener,
            client: crate::token::http_client()?,
            id: uuid::Uuid::new_v4().to_string(),
            url: url.into(),
            state,
            verifier,
            nonce,
            redirect,
            started: Instant::now(),
        })
    }

    pub fn poll(&mut self, home: &Path) -> Result<Option<Value>, String> {
        let client = self.client.clone();
        self.poll_at(&client, TOKEN_URL, |tokens| {
            crate::token::save_token(home, &tokens.to_string())
        })
    }

    fn poll_at(
        &mut self,
        client: &Client,
        token_endpoint: &str,
        save: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<Option<Value>, String> {
        if self.started.elapsed() > Duration::from_secs(600) {
            return Err("登录已超时，请重新打开网页登录".into());
        }
        let (mut stream, peer) = match self.listener.accept() {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(_) => return Err("登录回调连接失败，请重新登录".into()),
        };
        if !peer.ip().is_loopback() {
            return Ok(None);
        }
        stream.set_read_timeout(Some(Duration::from_secs(1))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(1))).ok();
        let mut request = String::new();
        {
            let mut reader = BufReader::new((&stream).take(16 * 1024));
            if reader.read_line(&mut request).is_err() {
                return Ok(None);
            }
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => {
                        respond(&mut stream, 400, "回调请求不完整");
                        return Ok(None);
                    }
                    Ok(_) if line == "\r\n" => break,
                    Ok(_) => {}
                }
            }
        }
        let mut fields = request.split_whitespace();
        if fields.next() != Some("GET") {
            respond(&mut stream, 405, "只接受登录回调");
            return Ok(None);
        }
        let Some(target) = fields.next().filter(|p| p.starts_with('/')) else {
            respond(&mut stream, 400, "回调请求无效");
            return Ok(None);
        };
        let Ok(url) = url::Url::parse(&format!("http://localhost{target}")) else {
            respond(&mut stream, 400, "回调地址无效");
            return Ok(None);
        };
        if url.path() != "/auth/callback" {
            respond(&mut stream, 404, "页面不存在");
            return Ok(None);
        }
        let pairs: Vec<_> = url.query_pairs().collect();
        let find = |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_ref())
        };
        if find("state") != Some(self.state.as_str()) {
            respond(&mut stream, 400, "登录校验失败，请使用本次打开的登录页面");
            return Ok(None);
        }
        if find("error").is_some() {
            respond(&mut stream, 400, "登录未完成，请返回 QuotaMonitor 重试");
            return Err("网页登录未完成，请重试".into());
        }
        let Some(code) = find("code").filter(|c| !c.is_empty()) else {
            respond(&mut stream, 400, "缺少登录授权码");
            return Ok(None);
        };
        let result = exchange_at(
            client,
            token_endpoint,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", CLIENT_ID),
                ("code", code),
                ("redirect_uri", self.redirect.as_str()),
                ("code_verifier", self.verifier.as_str()),
            ],
        )
        .and_then(|tokens| {
            validate_identity(&tokens, &self.nonce)?;
            // A browser success page must mean the credentials were committed
            // to the selected monitor account, not only that OAuth succeeded.
            save(&tokens).map_err(|e| format!("网页登录已授权，但登录信息保存失败：{e}"))?;
            Ok(tokens)
        });
        match result {
            Ok(tokens) => {
                respond(
                    &mut stream,
                    200,
                    "登录信息已保存。请返回 QuotaMonitor 查看额度查询结果。此页面可以关闭。",
                );
                Ok(Some(tokens))
            }
            Err(e) => {
                respond(
                    &mut stream,
                    502,
                    &format!("登录未完成：{e}。请返回 QuotaMonitor 查看详情。"),
                );
                Err(e)
            }
        }
    }
}

// The token endpoint is authenticated with TLS. Validate the ID-token binding
// as well as PKCE/state; never trust identity claims supplied by the callback.
fn validate_identity(tokens: &Value, nonce: &str) -> Result<(), String> {
    let id = tokens
        .get("id_token")
        .and_then(Value::as_str)
        .ok_or("登录响应缺少账号身份，请重新登录")?;
    let claims = crate::token::claims(id);
    let audience = &claims["aud"];
    let valid_audience = audience.as_str() == Some(CLIENT_ID)
        || audience
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(CLIENT_ID)));
    if claims["iss"].as_str().map(|s| s.trim_end_matches('/')) != Some("https://auth.openai.com")
        || !valid_audience
        || claims["nonce"].as_str() != Some(nonce)
        || claims["exp"]
            .as_i64()
            .is_none_or(|e| e <= crate::model::now())
    {
        return Err("登录账号身份校验失败，请重新打开网页登录".into());
    }
    Ok(())
}

fn respond(stream: &mut TcpStream, status: u16, text: &str) {
    let text = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let body = format!(
        "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>QuotaMonitor 登录</title><body><h1>QuotaMonitor</h1><p>{text}</p></body></html>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Result\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn exchange_at(client: &Client, endpoint: &str, form: &[(&str, &str)]) -> Result<Value, String> {
    let response = client
        .post(endpoint)
        .form(form)
        .send()
        .map_err(|_| "登录连接失败，请检查网络或本机代理")?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            400 | 401 | 403 => "登录授权已失效，请重新网页登录".into(),
            429 => "登录请求过于频繁，请稍后重试".into(),
            n => format!("登录服务暂不可用（HTTP {n}），请稍后重试"),
        });
    }
    let mut bytes = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "登录响应读取失败")?;
    if bytes.len() > 1024 * 1024 {
        return Err("登录响应过大".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "登录响应格式无效")?;
    if value["access_token"].as_str().is_none_or(|s| s.is_empty()) {
        return Err("登录响应缺少 token".into());
    }
    Ok(value)
}

pub fn refresh(client: &Client, refresh_token: &str) -> Result<Value, String> {
    exchange_at(
        client,
        TOKEN_URL,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::thread;
    #[test]
    fn browser_callback_exchanges_pkce_code_and_rejects_wrong_state() {
        callback_flow(false);
    }
    #[test]
    fn browser_does_not_report_success_when_account_save_fails() {
        callback_flow(true);
    }
    fn callback_flow(save_fails: bool) {
        let home =
            std::env::temp_dir().join(format!("quota-browser-save-{}", uuid::Uuid::new_v4()));
        if save_fails {
            std::fs::create_dir_all(home.join("auth.json")).unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let redirect = format!("http://localhost:{port}/auth/callback");
        let mut login = BrowserLogin::with_listener(listener, redirect.clone()).unwrap();
        let url = url::Url::parse(&login.url).unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            params["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(login.verifier.as_bytes()))
        );
        assert_eq!(params["redirect_uri"], redirect);
        assert_eq!(params["scope"], "openid profile email offline_access");
        let token_server = TcpListener::bind("127.0.0.1:0").unwrap();
        let token_endpoint = format!("http://{}/token", token_server.local_addr().unwrap());
        let nonce = login.nonce.clone();
        let expected_verifier = login.verifier.clone();
        let server = thread::spawn(move || {
            let (mut socket, _) = token_server.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let form: std::collections::HashMap<_, _> =
                url::form_urlencoded::parse(&body).into_owned().collect();
            assert_eq!(form["code_verifier"], expected_verifier);
            assert_eq!(form["code"], "fixture-code");
            assert_eq!(form["grant_type"], "authorization_code");
            let identity = json!({"iss":"https://auth.openai.com","aud":CLIENT_ID,"nonce":nonce,"exp":crate::model::now()+600});
            let id_token = format!(
                "test.{}.test",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&identity).unwrap())
            );
            let body=json!({"access_token":"browser-token","refresh_token":"refresh-token","id_token":id_token,"expires_in":3600}).to_string();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let http = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        for state in ["wrong-state".to_string(), login.state.clone()] {
            let request = format!(
                "GET /auth/callback?code=fixture-code&state={state} HTTP/1.1\r\nHost: localhost\r\n\r\n"
            );
            let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket.write_all(request.as_bytes()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let result = loop {
                let result = login.poll_at(&http, &token_endpoint, |tokens| {
                    socket.set_nonblocking(true).unwrap();
                    assert_eq!(
                        socket.peek(&mut [0u8; 1]).unwrap_err().kind(),
                        std::io::ErrorKind::WouldBlock,
                        "success must wait for account persistence"
                    );
                    socket.set_nonblocking(false).unwrap();
                    crate::token::save_token(&home, &tokens.to_string())
                });
                if !matches!(&result, Ok(None)) {
                    break result;
                }
                socket.set_nonblocking(true).unwrap();
                let ready = socket.peek(&mut [0u8; 1]).is_ok();
                socket.set_nonblocking(false).unwrap();
                if ready {
                    break result;
                }
                assert!(Instant::now() < deadline, "callback did not get accepted");
                thread::sleep(Duration::from_millis(5));
            };
            if state == "wrong-state" {
                assert!(result.unwrap().is_none());
            } else if save_fails {
                assert!(result.unwrap_err().contains("保存失败"));
            } else {
                assert_eq!(result.unwrap().unwrap()["access_token"], "browser-token");
                let saved: Value =
                    serde_json::from_slice(&std::fs::read(home.join("auth.json")).unwrap())
                        .unwrap();
                assert_eq!(saved["tokens"]["access_token"], "browser-token");
                assert_eq!(saved["tokens"]["refresh_token"], "refresh-token");
            }
            let mut response = String::new();
            socket.read_to_string(&mut response).unwrap();
            assert!(!response.contains("browser-token"));
            if state != "wrong-state" {
                if save_fails {
                    assert!(response.starts_with("HTTP/1.1 502"));
                    assert!(!response.contains("登录信息已保存"));
                } else {
                    assert!(response.starts_with("HTTP/1.1 200"));
                    assert!(response.contains("登录信息已保存"));
                }
            }
        }
        server.join().unwrap();
        if home.exists() {
            std::fs::remove_dir_all(home).unwrap();
        }
    }
    #[test]
    fn identity_must_match_the_current_browser_login() {
        let identity = json!({"iss":"https://auth.openai.com","aud":CLIENT_ID,"nonce":"other-login","exp":crate::model::now()+600});
        let token = format!(
            "test.{}.test",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&identity).unwrap())
        );
        assert!(validate_identity(&json!({"id_token":token}), "this-login").is_err());
    }
}
