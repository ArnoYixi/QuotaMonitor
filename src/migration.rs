//! Relocate this application's old storage, without importing other programs' auth.
use crate::{cleanup, store, token};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

pub fn open_default() -> Result<(PathBuf, File), String> {
    let preferred = store::default_dir();
    let base = preferred.parent().ok_or("无法定位本地保存位置")?;
    let legacy: Vec<_> = ["QuotaMonitor", "QuotaMonitori"]
        .into_iter()
        .filter_map(|name| {
            directories::ProjectDirs::from("com", "ArnoYixi", name)
                .map(|d| d.data_local_dir().to_path_buf())
        })
        .filter(|p| p != &preferred)
        .collect();
    relocate(&preferred, &legacy, base)
}

fn has_state(path: &Path) -> bool {
    path.join("settings.json").exists() || path.join("accounts").exists()
}

fn relocate(preferred: &Path, legacy: &[PathBuf], base: &Path) -> Result<(PathBuf, File), String> {
    if has_state(preferred) {
        let lock = store::lock_instance(preferred)?;
        return Ok((preferred.to_path_buf(), lock));
    }
    for source in legacy.iter().filter(|p| has_state(p)) {
        let boundary = base.canonicalize().map_err(|_| "无法定位本地保存位置")?;
        cleanup::checked_path(&boundary, source)?;
        if preferred.exists() {
            let mut entries = fs::read_dir(preferred).map_err(|_| "无法检查新的保存位置")?;
            if entries.next().is_some() {
                return Err("新的保存位置已有其他文件，旧账号已保留，未覆盖任何文件".into());
            }
            fs::remove_dir(preferred).map_err(|_| "无法准备新的保存位置，旧账号已保留")?;
        }
        // Finish preparation with the old instance locked. Windows requires
        // closing its lock file before the atomic directory rename.
        let lock = store::lock_instance(source)?;
        let settings = store::load_settings(source)?;
        if settings.codex_path.trim().is_empty() {
            cleanup::legacy_data(source)?;
        }
        let accounts = source.join("accounts");
        if accounts.exists() {
            cleanup::checked_path(
                &source.canonicalize().map_err(|_| "无法定位旧保存位置")?,
                &accounts,
            )?;
            for entry in fs::read_dir(&accounts).map_err(|_| "无法检查旧账号")? {
                let entry = entry.map_err(|_| "无法检查旧账号")?;
                if uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_ok()
                    && entry.path().is_dir()
                {
                    cleanup::checked_path(
                        &accounts.canonicalize().map_err(|_| "无法定位旧账号")?,
                        &entry.path(),
                    )?;
                    token::prepare_relocation(&entry.path())?;
                }
            }
        }
        drop(lock);
        fs::rename(source, preferred)
            .map_err(|_| "无法迁移保存位置，旧账号和设置已保留；请关闭旧版后重试")?;
        let lock = store::lock_instance(preferred)?;
        // Historical cleanup reports are unnecessary once the old components
        // are removed. Never recreate them in the new application folder.
        let report = preferred.join("upgrade-cleanup.json");
        if report.is_file() {
            let _ = fs::remove_file(report);
        }
        prune_empty_parents(source.parent(), base);
        return Ok((preferred.to_path_buf(), lock));
    }
    let lock = store::lock_instance(preferred)?;
    Ok((preferred.to_path_buf(), lock))
}

fn prune_empty_parents(mut path: Option<&Path>, boundary: &Path) {
    while let Some(current) = path {
        if current == boundary || !current.starts_with(boundary) {
            break;
        }
        // remove_dir only removes an empty directory; sibling apps and files
        // under an old vendor folder are therefore always preserved.
        if fs::remove_dir(current).is_err() {
            break;
        }
        path = current.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!("quota-relocate-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn fixture(base: &Path) -> (PathBuf, PathBuf, String) {
        let source = base.join("OldPublisher/QuotaMonitor/data");
        let preferred = base.join("QuotaMonitor");
        let id = uuid::Uuid::new_v4().to_string();
        fs::create_dir_all(source.join("accounts").join(&id)).unwrap();
        let settings = store::Settings {
            accounts: vec![store::AccountConfig {
                id: id.clone(),
                alias: "saved-account".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        store::atomic_json(&source.join("settings.json"), &settings).unwrap();
        token::save_token(&source.join("accounts").join(&id), "fixture-access").unwrap();
        fs::write(source.join("upgrade-cleanup.json"), "obsolete report").unwrap();
        (source, preferred, id)
    }
    #[test]
    fn relocates_accounts_and_settings_with_the_instance_lock_held() {
        let base = root();
        let (source, preferred, id) = fixture(&base);
        let original = fs::read(source.join("accounts").join(&id).join("auth.json")).unwrap();
        let (path, lock) = relocate(&preferred, std::slice::from_ref(&source), &base).unwrap();
        assert_eq!(path, preferred);
        assert!(!source.exists());
        assert!(!base.join("OldPublisher").exists());
        assert!(!path.join("data").exists());
        assert!(!path.join("upgrade-cleanup.json").exists());
        assert_eq!(
            fs::read(path.join("accounts").join(&id).join("auth.json")).unwrap(),
            original
        );
        assert_eq!(
            store::load_settings(&path).unwrap().accounts[0].alias,
            "saved-account"
        );
        assert!(store::lock_instance(&path).is_err());
        drop(lock);
        let (_, lock) = relocate(&preferred, &[source], &base).unwrap();
        drop(lock);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn running_old_version_prevents_relocation_without_losing_data() {
        let base = root();
        let (source, preferred, id) = fixture(&base);
        let lock = store::lock_instance(&source).unwrap();
        assert!(relocate(&preferred, std::slice::from_ref(&source), &base).is_err());
        assert!(source.join("accounts").join(id).join("auth.json").exists());
        assert!(!preferred.exists());
        drop(lock);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn existing_new_accounts_are_not_overwritten() {
        let base = root();
        let (source, preferred, _) = fixture(&base);
        fs::create_dir_all(&preferred).unwrap();
        store::atomic_json(
            &preferred.join("settings.json"),
            &store::Settings::default(),
        )
        .unwrap();
        let (_, lock) = relocate(&preferred, std::slice::from_ref(&source), &base).unwrap();
        assert!(
            store::load_settings(&preferred)
                .unwrap()
                .accounts
                .is_empty()
        );
        assert!(source.join("settings.json").exists());
        drop(lock);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn empty_destination_is_supported_and_other_vendor_files_survive() {
        let base = root();
        let (source, preferred, _) = fixture(&base);
        fs::create_dir_all(&preferred).unwrap();
        let sibling = base.join("OldPublisher/other-app.txt");
        fs::write(&sibling, "keep").unwrap();
        let (_, lock) = relocate(&preferred, &[source], &base).unwrap();
        assert_eq!(fs::read_to_string(sibling).unwrap(), "keep");
        drop(lock);
        fs::remove_dir_all(base).unwrap();
    }
}
