use eframe::egui;
use std::{path::Path, sync::mpsc::Sender};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[derive(Clone, Copy, Debug)]
pub enum TrayAction {
    Show,
    Hide,
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
    let hide = MenuItem::new("隐藏悬浮窗", true, None);
    let refresh = MenuItem::new("刷新全部", true, None);
    let settings = MenuItem::new("设置", true, None);
    let quit = MenuItem::new("退出 QuotaMonitori", true, None);
    menu.append_items(&[
        &show,
        &hide,
        &refresh,
        &settings,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .map_err(|e| e.to_string())?;
    let ids = [
        show.id().clone(),
        hide.id().clone(),
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
                    TrayAction::Hide,
                    TrayAction::Refresh,
                    TrayAction::Settings,
                    TrayAction::Quit,
                ][i],
            );
            // A hidden/occluded eframe window may not run App::update on a
            // repaint alone. Wake the native window before consuming the queue.
            if i != 1 {
                wake_window();
                event_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
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
            wake_window();
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
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

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Edge {
    #[default]
    Free,
    Top,
    Bottom,
    Left,
    Right,
}
pub struct DesktopGeometry {
    pub window: egui::Rect,
    pub work: egui::Rect,
    pub dragging: bool,
}
impl DesktopGeometry {
    pub fn edge(&self, scale: f32) -> Edge {
        let gap = 40.0 * scale;
        if (self.window.top() - self.work.top()).abs() <= gap {
            Edge::Top
        } else if (self.window.bottom() - self.work.bottom()).abs() <= gap {
            Edge::Bottom
        } else if (self.window.left() - self.work.left()).abs() <= gap {
            Edge::Left
        } else if (self.window.right() - self.work.right()).abs() <= gap {
            Edge::Right
        } else {
            Edge::Free
        }
    }
}

#[cfg(windows)]
fn main_window() -> windows_sys::Win32::Foundation::HWND {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::{
        Foundation::*, System::Threading::GetCurrentProcessId, UI::WindowsAndMessaging::*,
    };
    static WINDOW: AtomicIsize = AtomicIsize::new(0);
    unsafe extern "system" fn visit(hwnd: HWND, result: LPARAM) -> i32 {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == GetCurrentProcessId() {
                let mut title = [0u16; 128];
                let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
                if String::from_utf16_lossy(&title[..len as usize]).starts_with("QuotaMonitori ·")
                {
                    *(result as *mut HWND) = hwnd;
                    return 0;
                }
            }
            1
        }
    }
    unsafe {
        let cached = WINDOW.load(Ordering::Relaxed) as HWND;
        if !cached.is_null() && IsWindow(cached) != 0 {
            return cached;
        }
        let mut hwnd = std::ptr::null_mut();
        EnumWindows(Some(visit), &mut hwnd as *mut HWND as isize);
        WINDOW.store(hwnd as isize, Ordering::Relaxed);
        hwnd
    }
}

fn wake_window() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = main_window();
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
            PostMessageW(hwnd, WM_NULL, 0, 0);
        }
    }
}

pub fn desktop_geometry(ctx: &egui::Context) -> Option<DesktopGeometry> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*, UI::WindowsAndMessaging::*};
        let hwnd = main_window();
        if hwnd.is_null() {
            return None;
        }
        let mut rect: RECT = std::mem::zeroed();
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetWindowRect(hwnd, &mut rect) == 0
            || GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) == 0
        {
            return None;
        }
        let convert = |r: RECT| {
            egui::Rect::from_min_max(
                egui::pos2(r.left as f32, r.top as f32),
                egui::pos2(r.right as f32, r.bottom as f32),
            )
        };
        let _ = ctx;
        Some(DesktopGeometry {
            window: convert(rect),
            work: convert(info.rcWork),
            dragging: windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(1) < 0,
        })
    }
    #[cfg(not(windows))]
    ctx.input(|i| {
        Some(DesktopGeometry {
            window: i.viewport().outer_rect? * ctx.pixels_per_point(),
            work: egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                i.viewport().monitor_size? * ctx.pixels_per_point(),
            ),
            dragging: i.pointer.primary_down(),
        })
    })
}

pub fn resize_overlay(
    ctx: &egui::Context,
    size: egui::Vec2,
    edge: Edge,
    geometry: Option<&DesktopGeometry>,
) {
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    #[cfg(windows)]
    if let Some(g) = geometry {
        let size = size * ctx.pixels_per_point();
        let mut pos = g.window.min;
        match edge {
            Edge::Top => pos.y = g.work.top(),
            Edge::Bottom => pos.y = g.work.bottom() - size.y,
            Edge::Left => pos.x = g.work.left(),
            Edge::Right => pos.x = g.work.right() - size.x,
            Edge::Free => {}
        }
        pos.x = pos
            .x
            .clamp(g.work.left(), (g.work.right() - size.x).max(g.work.left()));
        pos.y = pos
            .y
            .clamp(g.work.top(), (g.work.bottom() - size.y).max(g.work.top()));
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            let hwnd = main_window();
            if !hwnd.is_null() {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    pos.x.round() as i32,
                    pos.y.round() as i32,
                    size.x.round() as i32,
                    size.y.round() as i32,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (edge, geometry);
}

// Sample only a few transparent pixels adjacent to the text, locally. Never
// capture, store or transmit a screenshot. The outline covers mixed backgrounds.
pub fn text_color(ctx: &egui::Context, rect: egui::Rect) -> egui::Color32 {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{Foundation::POINT, Graphics::Gdi::*};
        let hwnd = main_window();
        if !hwnd.is_null() {
            let dc = GetDC(std::ptr::null_mut());
            if !dc.is_null() {
                let scale = ctx.pixels_per_point();
                let mut origin = POINT { x: 0, y: 0 };
                ClientToScreen(hwnd, &mut origin);
                let mut sum = 0.0;
                let mut count = 0;
                for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    let x = origin.x + ((rect.left() + rect.width() * t) * scale) as i32;
                    let y = origin.y + ((rect.top() - 2.0) * scale) as i32;
                    let c = GetPixel(dc, x, y);
                    if c != 0xffffffff {
                        sum += (c & 255) as f32 * 0.2126
                            + ((c >> 8) & 255) as f32 * 0.7152
                            + ((c >> 16) & 255) as f32 * 0.0722;
                        count += 1;
                    }
                }
                ReleaseDC(std::ptr::null_mut(), dc);
                if count > 0 {
                    return if sum / count as f32 > 150.0 {
                        egui::Color32::BLACK
                    } else {
                        egui::Color32::WHITE
                    };
                }
            }
        }
    }
    let _ = rect;
    if ctx.style().visuals.dark_mode {
        egui::Color32::WHITE
    } else {
        egui::Color32::BLACK
    }
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
