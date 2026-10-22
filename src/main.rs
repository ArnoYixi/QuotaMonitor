#![cfg_attr(windows, windows_subsystem = "windows")]
mod desktop;
mod platform;
mod ui;
use eframe::egui;
use quotamonitori::store;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pair) = args.windows(2).find(|w| w[0] == "--self-test") {
        let result = self_test();
        let report = match &result {
            Ok(()) => {
                serde_json::json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"runtime":quotamonitori::runtime::RUNTIME_VERSION,"checks":["bundle_sha256","extraction","child_process","initialize","account_read_logged_out"]})
            }
            Err(e) => serde_json::json!({"ok":false,"error":e}),
        };
        let saved = std::fs::write(&pair[1], serde_json::to_vec_pretty(&report).unwrap());
        std::process::exit(if result.is_ok() && saved.is_ok() {
            0
        } else {
            1
        });
    }
    if let Err(e) = run() {
        platform::show_error(&e);
    }
}
fn self_test() -> Result<(), String> {
    use quotamonitori::{rpc::Rpc, runtime};
    use std::sync::{Arc, atomic::AtomicBool};
    let root = std::env::temp_dir().join(format!("quotamonitori-probe-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let backend = runtime::resolve(&root, "")?;
        let mut rpc = Rpc::spawn(
            &backend,
            &root.join("account"),
            true,
            Arc::new(AtomicBool::new(false)),
        )
        .map_err(|e| e.message)?;
        let account = rpc
            .request(
                "account/read",
                serde_json::json!({"refreshToken":false}),
                std::time::Duration::from_secs(25),
            )
            .map_err(|e| e.message)?;
        if !account
            .get("account")
            .is_some_and(serde_json::Value::is_null)
        {
            return Err("隔离自检目录应没有登录账号".into());
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(root);
    result
}
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let demo = args.iter().any(|a| a == "--demo");
    let mut dir = args
        .windows(2)
        .find(|w| w[0] == "--data-dir")
        .map(|w| std::path::PathBuf::from(&w[1]))
        .unwrap_or_else(store::default_dir);
    if demo {
        dir = dir.join("demo");
    }
    let lock = store::lock_instance(&dir)?;
    let mut settings = store::load_settings(&dir)?;
    if let Some(w) = args.windows(2).find(|w| w[0] == "--view") {
        settings.view = w[1].clone();
    }
    settings.validate()?;
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("QuotaMonitori · 额度监听")
        .with_decorations(false)
        .with_transparent(true)
        .with_has_shadow(false)
        .with_taskbar(false)
        .with_resizable(false)
        .with_inner_size([360.0, 220.0])
        .with_always_on_top()
        .with_icon(egui::IconData {
            rgba: platform::icon_rgba(64),
            width: 64,
            height: 64,
        });
    if let Some(p) = settings.position {
        viewport = viewport.with_position(p);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        multisampling: 0,
        vsync: true,
        ..Default::default()
    };
    desktop::run(options, move |cc, wake| {
        ui::App::new(cc, dir, settings, demo, lock, wake)
    })
}
