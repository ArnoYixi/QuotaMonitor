use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

#[derive(Clone, Debug)]
pub struct BackendCommand {
    pub executable: PathBuf,
    pub prefix: Vec<String>,
}
#[derive(Clone, Debug)]
pub enum Backend {
    Token,
    AppServer(BackendCommand),
}
impl From<BackendCommand> for Backend {
    fn from(command: BackendCommand) -> Self {
        Self::AppServer(command)
    }
}
impl Backend {
    pub fn uses_tokens(&self) -> bool {
        matches!(self, Self::Token)
    }
}
pub fn resolve(dir: &Path, override_path: &str) -> Result<Backend, String> {
    if override_path.trim().is_empty() {
        return Ok(Backend::Token);
    }
    resolve_app_server(dir, override_path).map(Backend::AppServer)
}
pub fn resolve_app_server(dir: &Path, override_path: &str) -> Result<BackendCommand, String> {
    if !override_path.trim().is_empty() {
        let p = PathBuf::from(override_path.trim());
        if !p.is_absolute() || !p.is_file() {
            return Err("官方组件路径必须是存在的可执行文件绝对路径".into());
        }
        return Ok(command_for(p));
    }
    if !RUNTIME_ARCHIVE.is_empty() {
        return unpack(dir).map(command_for);
    }
    for name in [
        if cfg!(windows) {
            "codex-app-server.exe"
        } else {
            "codex-app-server"
        },
        if cfg!(windows) { "codex.exe" } else { "codex" },
    ] {
        if let Some(paths) = std::env::var_os("PATH") {
            for base in std::env::split_paths(&paths) {
                let p = base.join(name);
                if p.is_file() {
                    return Ok(command_for(p));
                }
            }
        }
    }
    Err("未找到官方组件。请使用内置组件的完整 EXE，或在设置中指定 codex 可执行文件。".into())
}
pub fn command_for(path: PathBuf) -> BackendCommand {
    let standalone = path
        .file_name()
        .is_some_and(|s| s.to_string_lossy().contains("app-server"));
    BackendCommand {
        executable: path,
        prefix: if standalone {
            vec![]
        } else {
            vec!["app-server".into()]
        },
    }
}
fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|_| "无法读取官方组件")?;
    let mut h = Sha256::new();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b).map_err(|_| "无法验证官方组件")?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn find_server(base: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(base).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(v) = find_server(&p) {
                return Some(v);
            }
        } else {
            let n = p.file_name()?.to_string_lossy();
            if n == "codex-app-server" || n == "codex-app-server.exe" {
                return Some(p);
            }
        }
    }
    None
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    archive_hash: String,
    files: Vec<(PathBuf, String)>,
}
fn unpack(dir: &Path) -> Result<PathBuf, String> {
    let runtime = dir.join("runtime").join(RUNTIME_VERSION);
    let manifest_path = runtime.join("verified.json");
    if manifest_path.is_file() {
        let valid = (|| -> Option<PathBuf> {
            let m: Manifest = serde_json::from_slice(&fs::read(&manifest_path).ok()?).ok()?;
            if m.archive_hash != RUNTIME_SHA256 || m.files.is_empty() {
                return None;
            }
            for (relative, hash) in &m.files {
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|c| !matches!(c, std::path::Component::Normal(_)))
                {
                    return None;
                }
                if sha256_file(&runtime.join(relative)).ok()?.as_str() != hash {
                    return None;
                }
            }
            find_server(&runtime)
        })();
        if let Some(p) = valid {
            return Ok(p);
        }
        return Err(
            "本地官方组件校验失败。关闭程序后删除数据目录内的 runtime 文件夹，再启动修复。".into(),
        );
    }
    if format!("{:x}", Sha256::digest(RUNTIME_ARCHIVE)) != RUNTIME_SHA256 {
        return Err("内置官方组件完整性校验失败".into());
    }
    crate::store::ensure_private_dir(&runtime).map_err(|_| "无法创建官方组件目录")?;
    let decoder =
        zstd::stream::read::Decoder::new(RUNTIME_ARCHIVE).map_err(|_| "官方组件解压失败")?;
    let mut archive = tar::Archive::new(decoder);
    let mut files = vec![];
    for entry in archive.entries().map_err(|_| "官方组件归档损坏")? {
        let mut entry = entry.map_err(|_| "无法读取官方组件归档")?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            return Err("官方组件归档包含不支持的链接".into());
        }
        let relative = entry.path().map_err(|_| "归档路径无效")?.into_owned();
        if !entry
            .unpack_in(&runtime)
            .map_err(|_| "官方组件解压失败，请检查磁盘空间")?
        {
            return Err("归档路径越界".into());
        }
        if kind.is_file() {
            let normalized: PathBuf = relative
                .components()
                .filter(|c| *c != std::path::Component::CurDir)
                .collect();
            #[cfg(unix)]
            if normalized.file_name().is_some_and(|n| {
                matches!(
                    n.to_str(),
                    Some("codex-app-server" | "codex-code-mode-host" | "rg" | "bwrap" | "zsh")
                )
            }) {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(runtime.join(&normalized), fs::Permissions::from_mode(0o700))
                    .map_err(|_| "无法设置组件执行权限")?;
            }
            files.push((normalized.clone(), sha256_file(&runtime.join(normalized))?));
        }
    }
    let server = find_server(&runtime).ok_or("归档中没有官方 App Server")?;
    crate::store::atomic_json(
        &manifest_path,
        &Manifest {
            archive_hash: RUNTIME_SHA256.into(),
            files,
        },
    )?;
    Ok(server)
}
pub fn trusted_auth_url(raw: &str) -> bool {
    url::Url::parse(raw).ok().is_some_and(|u| {
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none_or(|p| p == 443)
            && matches!(u.host_str(), Some("auth.openai.com" | "chatgpt.com"))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_official_auth_origins() {
        assert!(trusted_auth_url("https://auth.openai.com/codex/device"));
        for bad in [
            "http://auth.openai.com/",
            "https://auth.openai.com.evil.org/",
            "file:///tmp/a",
            "https://chatgpt.com@evil.org/",
            "https://evil.org@chatgpt.com/",
            "https://chatgpt.com:444/",
        ] {
            assert!(!trusted_auth_url(bad), "{bad}");
        }
    }
}
