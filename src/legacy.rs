//! One-time migration of credentials owned by older QuotaMonitor installations.
//! The namespace and path hash follow openai/codex's Apache-2.0 secrets format.
use std::path::Path;

#[cfg(not(windows))]
pub(crate) fn remember_home(_home: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn home_hash(home: &Path) -> String {
    use sha2::{Digest, Sha256};
    let canonical = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    format!(
        "{:x}",
        Sha256::digest(canonical.to_string_lossy().as_bytes())
    )
}

#[cfg(windows)]
fn stored_home_hash(home: &Path) -> Result<String, String> {
    let path = home.join("secrets/legacy-home.json");
    if !path.exists() {
        return Ok(home_hash(home));
    }
    let root = home.canonicalize().map_err(|_| "无法定位旧账号")?;
    let path = crate::cleanup::checked_path(&root, &path)?;
    let bytes = std::fs::read(path).map_err(|_| "无法读取旧账号密钥索引")?;
    if bytes.len() > 1024 {
        return Err("旧账号密钥索引无效".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "旧账号密钥索引无效")?;
    value["homeHash"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(String::from)
        .ok_or_else(|| "旧账号密钥索引无效".into())
}

#[cfg(windows)]
pub(crate) fn remember_home(home: &Path) -> Result<(), String> {
    if home.join("secrets/legacy-home.json").exists() {
        stored_home_hash(home)?;
        return Ok(());
    }
    if home.join("auth.json").is_file() && !home.join("secrets/codex_auth.age").exists() {
        return Ok(());
    }
    crate::store::atomic_json(
        &home.join("secrets/legacy-home.json"),
        &serde_json::json!({"homeHash":home_hash(home)}),
    )
}

pub(crate) fn load(home: &Path) -> Result<Option<String>, String> {
    // Never inspect a global Codex home or enumerate unrelated credentials.
    if home
        .parent()
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        != Some("accounts")
        || home
            .file_name()
            .and_then(|s| s.to_str())
            .is_none_or(|s| uuid::Uuid::parse_str(s).is_err())
    {
        return Ok(None);
    }
    load_owned_account(home)
}

#[cfg(not(windows))]
fn load_owned_account(_home: &Path) -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(windows)]
fn load_owned_account(home: &Path) -> Result<Option<String>, String> {
    use std::{fs, sync::Mutex};
    static MIGRATION: Mutex<()> = Mutex::new(());
    let _guard = MIGRATION
        .lock()
        .map_err(|_| "旧账号迁移暂不可用，请重新启动")?;
    let canonical = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let hash = stored_home_hash(home)?;
    let encrypted = home.join("secrets/codex_auth.age");
    if encrypted.is_file() {
        if !encrypted
            .canonicalize()
            .map_err(|_| "无法定位旧账号凭据")?
            .starts_with(&canonical)
        {
            return Err("旧账号凭据指向数据目录外，已跳过迁移".into());
        }
        let meta = fs::symlink_metadata(&encrypted).map_err(|_| "无法读取旧账号凭据")?;
        if meta.len() > 1024 * 1024 || meta.file_type().is_symlink() {
            return Err("旧账号凭据文件无效，原文件已保留".into());
        }
        let account = format!("secrets|{}", &hash[..16]);
        let passphrase = read_key("codex", &account)?
            .ok_or("旧账号系统密钥不可用，原凭据已保留；请重新网页登录")?;
        let ciphertext = fs::read(encrypted).map_err(|_| "无法读取旧账号凭据")?;
        return decode(&ciphertext, &passphrase).map(Some);
    }
    // Releases predating encrypted namespace files used a direct keyring entry.
    read_key("Codex Auth", &format!("cli|{}", &hash[..16]))
}

#[cfg(windows)]
fn read_key(service: &str, account: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(service, account).map_err(|_| "无法访问旧账号系统凭据")?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("系统凭据库暂不可用，旧登录已保留；请稍后重试或网页登录".into()),
    }
}

#[cfg(windows)]
fn decode(ciphertext: &[u8], passphrase: &str) -> Result<String, String> {
    let mut identity =
        age::scrypt::Identity::new(age::secrecy::SecretString::from(passphrase.to_string()));
    identity.set_max_work_factor(20);
    let plaintext = age::decrypt(&identity, ciphertext)
        .map_err(|_| "旧账号凭据解密失败，原文件已保留；请重新网页登录")?;
    let value: serde_json::Value =
        serde_json::from_slice(&plaintext).map_err(|_| "旧账号凭据格式无效，原文件已保留")?;
    if value["version"].as_u64().is_some_and(|n| n > 1) {
        return Err("旧账号凭据来自更新版本，原文件已保留".into());
    }
    value
        .pointer("/secrets/global~1CODEX_AUTH")
        .and_then(serde_json::Value::as_str)
        .map(String::from)
        .ok_or_else(|| "旧账号未保存可复用的登录，请重新网页登录".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn original_key_index_survives_relocation_without_storing_the_old_path() {
        let root = std::env::temp_dir().join(format!("quota-home-key-{}", uuid::Uuid::new_v4()));
        let original = root
            .join("old/accounts")
            .join(uuid::Uuid::new_v4().to_string());
        let moved = root
            .join("new/accounts")
            .join(original.file_name().unwrap());
        std::fs::create_dir_all(original.join("secrets")).unwrap();
        std::fs::write(original.join("secrets/codex_auth.age"), b"opaque fixture").unwrap();
        let expected = home_hash(&original);
        remember_home(&original).unwrap();
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::rename(&original, &moved).unwrap();
        assert_ne!(home_hash(&moved), expected);
        assert_eq!(stored_home_hash(&moved).unwrap(), expected);
        let index = std::fs::read_to_string(moved.join("secrets/legacy-home.json")).unwrap();
        assert!(!index.contains("old"));
        assert!(!index.contains("accounts"));
        std::fs::write(
            moved.join("secrets/legacy-home.json"),
            r#"{"homeHash":"invalid"}"#,
        )
        .unwrap();
        assert!(stored_home_hash(&moved).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn global_auth_homes_are_never_inspected() {
        assert!(
            load(Path::new("C:/Users/example/.codex"))
                .unwrap()
                .is_none()
        );
        assert!(
            load(Path::new("C:/Users/example/AppData/Local/OtherApp/data"))
                .unwrap()
                .is_none()
        );
    }
    #[cfg(windows)]
    #[test]
    fn decrypts_only_the_saved_account_auth_record() {
        let passphrase = age::secrecy::SecretString::from("fixture-passphrase".to_string());
        let mut recipient = age::scrypt::Recipient::new(passphrase);
        recipient.set_work_factor(10);
        let auth = r#"{"tokens":{"access_token":"old-token","refresh_token":"old-refresh","account_id":"old-account"}}"#;
        let value = serde_json::json!({"version":1,"secrets":{"global/CODEX_AUTH":auth}});
        let ciphertext = age::encrypt(&recipient, value.to_string().as_bytes()).unwrap();
        assert_eq!(decode(&ciphertext, "fixture-passphrase").unwrap(), auth);
        assert!(decode(&ciphertext, "wrong-passphrase").is_err());
    }
}
