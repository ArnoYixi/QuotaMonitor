//! Check one explicitly supplied account directory without exposing credentials.
use quotamonitor::{model::Quota, token::TokenClient};
use serde_json::json;
use std::path::PathBuf;

fn main() {
    let home = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("account directory required"),
    );
    let result = (|| {
        let mut client = TokenClient::new(&home)?;
        if client.account()?["account"].is_null() {
            return Err("此账号目录没有登录凭据".to_string());
        }
        let raw = client.quota().map_err(|e| e.message)?;
        Quota::parse(raw, quotamonitor::model::now())
    })();
    let report = match result {
        Ok(quota) => {
            json!({"ok":true,"remainingPercent":quota.tightest().map(|(_,w)|w.remaining()),"fetchedAt":quota.fetched_at})
        }
        Err(message) => json!({"ok":false,"error":message}),
    };
    println!("{}", report);
}
