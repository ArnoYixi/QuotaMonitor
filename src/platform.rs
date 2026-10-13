use eframe::egui;
use std::{path::Path, sync::mpsc::Sender};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[derive(Clone, Copy, Debug)]
pub enum TrayAction {
    Show,
    Refresh,
    Settings,
    Quit,
    Available(bool),
}
pub struct Tray {
    #[cfg(windows)]
    _icon: Option<TrayIcon>,
}
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let mut data = vec![0; (size * size * 4) as usize];
    let s = size as f32;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - s / 2.0;
            let dy = y as f32 + 0.5 - s / 2.0;
            let r = (dx * dx + dy * dy).sqrt() / s;
            let a = (dy.atan2(dx) + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU);
            let color = if (0.28..0.44).contains(&r) {
                if a < std::f32::consts::TAU * 0.78 {
                    [47, 182, 135, 255]
                } else {
                    [74, 84, 93, 255]
                }
            } else {
                [0, 0, 0, 0]
            };
            let i = ((y * size + x) * 4) as usize;
            data[i..i + 4].copy_from_slice(&color);
        }
    }
    data
}
fn build_tray(tx: Sender<TrayAction>, ctx: egui::Context) -> Result<TrayIcon, String> {
    let menu = Menu::new();
    let show = MenuItem::new("显示悬浮窗", true, None);
    let refresh = MenuItem::new("刷新全部", true, None);
    let settings = MenuItem::new("设置", true, None);
    let quit = MenuItem::new("退出 QuotaMonitori", true, None);
    menu.append_items(&[
        &show,
        &refresh,
        &settings,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .map_err(|e| e.to_string())?;
    let ids = [
        show.id().clone(),
        refresh.id().clone(),
        settings.id().clone(),
        quit.id().clone(),
    ];
    let event_tx = tx.clone();
    let event_ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(i) = ids.iter().position(|id| *id == event.id) {
            let _ = event_tx.send(
                [
                    TrayAction::Show,
                    TrayAction::Refresh,
                    TrayAction::Settings,
                    TrayAction::Quit,
                ][i],
            );
            event_ctx.request_repaint();
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            let _ = tx.send(TrayAction::Show);
            ctx.request_repaint();
        }
    }));
    let icon = tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("QuotaMonitori · 额度监听")
        .with_icon(icon)
        .build()
        .map_err(|e| e.to_string())
}
impl Tray {
    pub fn start(tx: Sender<TrayAction>, ctx: egui::Context) -> Self {
        #[cfg(windows)]
        {
            let icon = build_tray(tx.clone(), ctx).ok();
            let _ = tx.send(TrayAction::Available(icon.is_some()));
            Self { _icon: icon }
        }
        #[cfg(target_os = "linux")]
        {
            std::thread::spawn(move || {
                if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() || gtk::init().is_err() {
                    let _ = tx.send(TrayAction::Available(false));
                    ctx.request_repaint();
                    return;
                }
                let icon = build_tray(tx.clone(), ctx.clone()).ok();
                let _ = tx.send(TrayAction::Available(icon.is_some()));
                ctx.request_repaint();
                if icon.is_some() {
                    gtk::main();
                }
                drop(icon);
            });
            Self {}
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = tx.send(TrayAction::Available(false));
            Self {}
        }
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        gtk::glib::MainContext::default().invoke(|| {
            if gtk::is_initialized() {
                gtk::main_quit();
            }
        });
    }
}

pub fn autostart(enabled: bool, exe: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::*};
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = root
            .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
            .map_err(|_| "无法更新开机启动设置")?;
        if enabled {
            key.set_value("QuotaMonitori", &format!("\"{}\"", exe.display()))
                .map_err(|_| "无法保存开机启动")?;
        } else {
            match key.delete_value("QuotaMonitori") {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("无法关闭开机启动".into()),
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let config = directories::BaseDirs::new()
            .ok_or("无法定位配置目录")?
            .config_dir()
            .join("autostart");
        let path = config.join("quotamonitori.desktop");
        if enabled {
            std::fs::create_dir_all(&config).map_err(|_| "无法创建启动目录")?;
            let raw = exe.to_string_lossy();
            if raw.contains(['\n', '\r']) {
                return Err("程序路径无效".into());
            }
            let quoted = raw
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('`', "\\`")
                .replace('$', "\\$")
                .replace('%', "%%");
            std::fs::write(path,format!("[Desktop Entry]\nType=Application\nName=QuotaMonitori\nExec=\"{quoted}\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n")).map_err(|_|"无法保存启动文件")?;
        } else if path.exists() {
            std::fs::remove_file(path).map_err(|_| "无法删除启动文件")?;
        }
    }
    Ok(())
}
pub fn show_error(message: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "QuotaMonitori".encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(windows))]
    eprintln!("QuotaMonitori: {message}");
}
