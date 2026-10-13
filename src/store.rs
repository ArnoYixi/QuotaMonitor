use crate::model::Quota;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountConfig {
    pub id: String,
    pub alias: String,
    pub enabled: bool,
    pub visible: bool,
    pub cached: Option<Quota>,
}
impl Default for AccountConfig {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            alias: "新账号".into(),
            enabled: true,
            visible: true,
            cached: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,
    pub accounts: Vec<AccountConfig>,
    pub view: String,
    pub interval: u64,
    pub opacity: f32,
    pub scale: f32,
    pub always_on_top: bool,
    pub lock_position: bool,
    pub light_theme: bool,
    pub autostart: bool,
    pub codex_path: String,
    pub ephemeral_login: bool,
    pub position: Option<[f32; 2]>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            accounts: vec![],
            view: "bar".into(),
            interval: 30,
            opacity: 0.98,
            scale: 1.0,
            always_on_top: true,
            lock_position: false,
            light_theme: false,
            autostart: false,
            codex_path: String::new(),
            ephemeral_login: false,
            position: None,
        }
    }
}
impl Settings {
    pub fn validate(&mut self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("设置来自更新版本，请升级程序".into());
        }
        let mut ids = std::collections::HashSet::new();
        for a in &self.accounts {
            if uuid::Uuid::parse_str(&a.id).is_err() || !ids.insert(&a.id) {
                return Err("账号配置无效或重复".into());
            }
        }
        if ![10, 30].contains(&self.interval) {
            self.interval = 30;
        }
        if self.view != "ring" {
            self.view = "bar".into();
        }
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.45, 1.0)
        } else {
            0.98
        };
        self.scale = if self.scale.is_finite() {
            self.scale.clamp(0.8, 1.8)
        } else {
            1.0
        };
        if self
            .position
            .is_some_and(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 100000.0))
        {
            self.position = None;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingReset {
    pub idempotency_key: String,
    pub credit_id: String,
    pub account_id: Option<String>,
    pub created_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetJournal {
    pub pending: Option<PendingReset>,
    pub last_outcome: Option<String>,
    pub last_completed_at: Option<i64>,
}
impl Default for ResetJournal {
    fn default() -> Self {
        Self {
            pending: None,
            last_outcome: None,
            last_completed_at: None,
        }
    }
}

pub fn default_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "ArnoYixi", "QuotaMonitori")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("QuotaMonitori-data"))
}
pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
/// Write+flush+atomic replace: a failed write cannot discard a pending redemption.
pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path.parent().ok_or("数据目录无效")?;
    ensure_private_dir(parent).map_err(|_| "无法创建数据目录")?;
    let temp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> io::Result<()> {
        let mut opts = OpenOptions::new();
        opts.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&temp)?;
        f.write_all(&serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
        f.sync_all()?;
        drop(f);
        #[cfg(not(windows))]
        fs::rename(&temp, path)?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
            };
            let a: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let b: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe {
                MoveFileExW(
                    a.as_ptr(),
                    b.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result.map_err(|_| "无法安全保存数据，请检查目录权限与剩余磁盘空间".into())
}
pub fn load_settings(dir: &Path) -> Result<Settings, String> {
    let path = dir.join("settings.json");
    if !path.exists() {
        return Ok(Settings::default());
    }
    let data = fs::read(&path).map_err(|_| "无法读取设置文件")?;
    let mut s: Settings =
        serde_json::from_slice(&data).map_err(|_| "设置文件损坏；已保留原文件，请先备份再修复")?;
    s.validate()?;
    Ok(s)
}
pub fn load_journal(dir: &Path) -> Result<ResetJournal, String> {
    let path = dir.join("reset-journal.json");
    if !path.exists() {
        return Ok(ResetJournal::default());
    }
    serde_json::from_slice(&fs::read(path).map_err(|_| "无法读取重置操作记录")?)
        .map_err(|_| "重置记录损坏，已禁止用卡以避免重复消耗".into())
}
pub fn save_journal(dir: &Path, j: &ResetJournal) -> Result<(), String> {
    atomic_json(&dir.join("reset-journal.json"), j)
}
pub fn lock_instance(dir: &Path) -> Result<File, String> {
    ensure_private_dir(dir).map_err(|_| "无法创建本地数据目录")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("instance.lock"))
        .map_err(|_| "无法打开进程锁")?;
    file.try_lock_exclusive()
        .map_err(|_| "QuotaMonitori 已在运行，请从系统托盘打开。")?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!("quotamonitori-test-{}", uuid::Uuid::new_v4()))
    }
    #[test]
    fn pending_operation_survives_restart() {
        let p = temp();
        let mut j = ResetJournal::default();
        j.pending = Some(PendingReset {
            idempotency_key: "same-key".into(),
            credit_id: "soon".into(),
            account_id: Some("account-a".into()),
            created_at: 1,
        });
        save_journal(&p, &j).unwrap();
        assert_eq!(load_journal(&p).unwrap().pending, j.pending);
        j.pending = None;
        j.last_outcome = Some("reset".into());
        save_journal(&p, &j).unwrap();
        assert!(load_journal(&p).unwrap().pending.is_none());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn corrupt_journal_never_resets_to_empty() {
        let p = temp();
        ensure_private_dir(&p).unwrap();
        fs::write(p.join("reset-journal.json"), b"{").unwrap();
        assert!(load_journal(&p).is_err());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn rejects_path_traversal_account_ids() {
        let mut s = Settings::default();
        s.accounts.push(AccountConfig {
            id: "../../.codex".into(),
            ..Default::default()
        });
        assert!(s.validate().is_err());
    }
    #[test]
    fn single_instance() {
        let p = temp();
        let first = lock_instance(&p).unwrap();
        assert!(lock_instance(&p).is_err());
        drop(first);
        assert!(lock_instance(&p).is_ok());
        fs::remove_dir_all(p).unwrap();
    }
}
