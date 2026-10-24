//! Remove only obsolete files owned by QuotaMonitor, retaining credential material.
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Default, Debug, serde::Serialize)]
pub struct Report {
    pub removed_bytes: u64,
    pub removed_entries: usize,
    pub failures: usize,
}

/// Called with the data-directory instance lock held and the token backend selected.
/// Resolve every deletion inside the supplied root and refuse directory links.
pub fn legacy_data(dir: &Path) -> Result<Report, String> {
    let root = fs::canonicalize(dir).map_err(|_| "无法定位数据目录，已跳过旧版清理")?;
    let mut report = Report::default();
    let accounts = root.join("accounts");
    let mut homes = Vec::new();
    if accounts.exists() {
        checked_path(&root, &accounts)?;
        for entry in fs::read_dir(&accounts).map_err(|_| "无法读取账号目录，已跳过旧版清理")?
        {
            let entry = entry.map_err(|_| "无法读取账号目录")?;
            if uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err() {
                continue;
            }
            let path = entry.path();
            if checked_path(&root, &path).is_err() || !path.is_dir() {
                report.failures += 1;
                continue;
            }
            homes.push(path);
        }
    }
    remove(&root, &root.join("runtime"), &mut report);
    for home in homes {
        let Ok(entries) = fs::read_dir(&home) else {
            report.failures += 1;
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                report.failures += 1;
                continue;
            };
            let name = entry.file_name();
            // Legacy encrypted credentials are retained so users can still recover
            // their login. Settings and pending reset journals are only a few KB.
            if matches!(
                name.to_str(),
                Some("auth.json" | "token.secret" | "secrets" | "reset-journal.json")
            ) {
                continue;
            }
            remove(&root, &entry.path(), &mut report);
        }
    }
    Ok(report)
}

pub(crate) fn checked_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "无法检查旧版文件")?;
    if meta.file_type().is_symlink() {
        return Err("已跳过数据目录中的链接".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("已跳过数据目录中的目录联接".into());
        }
    }
    let resolved = fs::canonicalize(path).map_err(|_| "无法定位旧版文件")?;
    if resolved == root || !resolved.starts_with(root) {
        return Err("旧版文件不在数据目录内，已跳过清理".into());
    }
    Ok(resolved)
}

fn size_without_links(path: &Path) -> std::io::Result<u64> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(std::io::Error::other("linked file"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(std::io::Error::other("reparse point"));
        }
    }
    if meta.is_file() {
        return Ok(meta.len());
    }
    let mut size = 0u64;
    for entry in fs::read_dir(path)? {
        size = size.saturating_add(size_without_links(&entry?.path())?);
    }
    Ok(size)
}

fn remove(root: &Path, path: &Path, report: &mut Report) {
    if !path.exists() {
        return;
    }
    let result = (|| -> Result<(), String> {
        let resolved = checked_path(root, path)?;
        // Traverse once before removing, refusing any nested link or reparse point.
        let bytes = size_without_links(&resolved).map_err(|_| "无法检查旧版目录中的文件")?;
        if resolved.is_dir() {
            fs::remove_dir_all(&resolved)
        } else {
            fs::remove_file(&resolved)
        }
        .map_err(|_| "部分旧版文件被占用，将在下次启动重试清理")?;
        report.removed_bytes = report.removed_bytes.saturating_add(bytes);
        report.removed_entries += 1;
        Ok(())
    })();
    if result.is_err() {
        report.failures += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upgrade_removes_runtime_and_agent_data_but_keeps_credentials() {
        let root = std::env::temp_dir().join(format!("quota-cleanup-{}", uuid::Uuid::new_v4()));
        let account = root.join("accounts").join(uuid::Uuid::new_v4().to_string());
        for dir in [
            root.join("runtime/0.162.1/bin"),
            account.join("plugins/cache"),
            account.join(".tmp/plugins"),
            account.join("cache"),
            account.join("secrets"),
        ] {
            fs::create_dir_all(dir).unwrap();
        }
        for file in [
            root.join("runtime/0.162.1/bin/codex.exe"),
            account.join("plugins/cache/plugin.zip"),
            account.join(".tmp/plugins/pack"),
            account.join("logs_2.sqlite"),
            account.join("cache/catalog.json"),
        ] {
            fs::write(file, b"unnecessary").unwrap();
        }
        for file in [
            root.join("settings.json"),
            account.join("auth.json"),
            account.join("secrets/codex_auth.age"),
            account.join("reset-journal.json"),
        ] {
            fs::write(file, b"keep").unwrap();
        }
        let report = legacy_data(&root).unwrap();
        assert_eq!(report.failures, 0);
        assert_eq!(report.removed_bytes, 55);
        assert!(!root.join("runtime").exists());
        assert!(!account.join("plugins").exists());
        assert!(!account.join("logs_2.sqlite").exists());
        for file in [
            root.join("settings.json"),
            account.join("auth.json"),
            account.join("secrets/codex_auth.age"),
            account.join("reset-journal.json"),
        ] {
            assert_eq!(fs::read(file).unwrap(), b"keep");
        }
        assert_eq!(legacy_data(&root).unwrap().removed_bytes, 0);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn obsolete_file_references_are_removed_without_opening_external_files() {
        let root =
            std::env::temp_dir().join(format!("quota-cleanup-link-{}", uuid::Uuid::new_v4()));
        let account = root.join("accounts").join(uuid::Uuid::new_v4().to_string());
        let cached_token = root.join("external/auth.json");
        fs::create_dir_all(cached_token.parent().unwrap()).unwrap();
        fs::write(&cached_token, r#"{"access_token":"test-only"}"#).unwrap();
        fs::create_dir_all(&account).unwrap();
        fs::write(
            account.join("token-source.json"),
            serde_json::json!({"path":cached_token}).to_string(),
        )
        .unwrap();
        assert_eq!(legacy_data(&root).unwrap().failures, 0);
        assert!(cached_token.is_file());
        assert!(!account.join("token-source.json").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cleanup_rejects_paths_outside_its_root() {
        let root =
            std::env::temp_dir().join(format!("quota-cleanup-boundary-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        assert!(checked_path(&root, &root).is_err());
        assert!(checked_path(&root, root.parent().unwrap()).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
