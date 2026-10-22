use crate::platform::{self, Tray, TrayAction};
use eframe::egui::{self, Color32, FontId, RichText, Sense, Stroke, Vec2};
use quotamonitori::{
    engine::{self, Command, Event, Status, ViewState, Worker},
    model::{Quota, now, quota_color, remaining_text, time_label},
    runtime::{self, BackendCommand},
    store::{self, AccountConfig, Settings},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq)]
enum Page {
    Monitor,
    Settings,
    Accounts,
    Detail(String),
}
enum Boot {
    Ready(Result<BackendCommand, String>),
}
pub struct App {
    dir: PathBuf,
    settings: Settings,
    demo: bool,
    _lock: std::fs::File,
    page: Page,
    states: HashMap<String, ViewState>,
    workers: HashMap<String, Worker>,
    backend: Option<BackendCommand>,
    events_tx: Sender<Event>,
    events_rx: Receiver<Event>,
    boot_rx: Receiver<Boot>,
    tray_rx: Receiver<TrayAction>,
    _tray: Tray,
    tray_available: bool,
    quitting: bool,
    notice: String,
    dirty: bool,
    last_save: Instant,
    last_demo_refresh: Instant,
    last_size: Vec2,
    need_login: Option<String>,
    context_account: Option<String>,
    edge: platform::Edge,
    columns: usize,
    was_monitor: bool,
    monitor_rect: Option<egui::Rect>,
    card_size: Vec2,
    monitor_size: Vec2,
    context_open: bool,
}
impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        dir: PathBuf,
        mut settings: Settings,
        demo: bool,
        lock: std::fs::File,
    ) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "cjk".into(),
            egui::FontData::from_static(include_bytes!("../assets/NotoSansSC-Subset.otf")).into(),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push("cjk".into());
        }
        cc.egui_ctx.set_fonts(fonts);
        cc.egui_ctx.set_zoom_factor(settings.scale);
        let (events_tx, events_rx) = mpsc::channel();
        let (boot_tx, boot_rx) = mpsc::channel();
        let (tray_tx, tray_rx) = mpsc::channel();
        platform::set_overlay_locked(settings.lock_position);
        let tray = Tray::start(tray_tx, cc.egui_ctx.clone());
        let mut states = HashMap::new();
        if demo && settings.accounts.is_empty() {
            for alias in ["主账号", "备用账号", "工作账号"] {
                settings.accounts.push(AccountConfig {
                    alias: alias.into(),
                    ..Default::default()
                });
            }
        }
        if demo {
            for (i, a) in settings.accounts.iter().enumerate() {
                states.insert(a.id.clone(), engine::demo_state(&a.id, i));
            }
        } else {
            for a in &settings.accounts {
                states.insert(a.id.clone(), ViewState::new(a.id.clone(), a.cached.clone()));
            }
            let root = dir.clone();
            let path = settings.codex_path.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                let r = runtime::resolve(&root, &path);
                let _ = boot_tx.send(Boot::Ready(r));
                ctx.request_repaint();
            });
        }
        let mut app = Self {
            dir,
            settings,
            demo,
            _lock: lock,
            page: Page::Monitor,
            states,
            workers: HashMap::new(),
            backend: None,
            events_tx,
            events_rx,
            boot_rx,
            tray_rx,
            _tray: tray,
            tray_available: false,
            quitting: false,
            notice: if demo {
                "演示模式：所有数据与操作均为模拟".into()
            } else {
                "正在准备官方登录组件…".into()
            },
            dirty: false,
            last_save: Instant::now(),
            last_demo_refresh: Instant::now(),
            last_size: Vec2::ZERO,
            need_login: None,
            context_account: None,
            edge: platform::Edge::Free,
            columns: 1,
            was_monitor: true,
            monitor_rect: None,
            card_size: Vec2::new(208.0, 43.0),
            monitor_size: Vec2::new(216.0, 51.0),
            context_open: false,
        };
        app.apply_style(&cc.egui_ctx);
        app.save();
        app
    }
    fn apply_style(&self, ctx: &egui::Context) {
        let mut v = if self.settings.light_theme {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        v.selection.bg_fill = Color32::from_rgb(29, 115, 92);
        v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
        v.window_corner_radius = egui::CornerRadius::same(12);
        ctx.set_visuals(v);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = Vec2::new(8.0, 8.0);
            s.spacing.button_padding = Vec2::new(9.0, 5.0);
        });
    }
    fn save(&mut self) {
        if let Err(e) = store::atomic_json(&self.dir.join("settings.json"), &self.settings) {
            self.notice = e;
        }
        self.dirty = false;
        self.last_save = Instant::now();
    }
    fn spawn(&mut self, id: &str, ctx: &egui::Context) {
        if self.demo || self.workers.contains_key(id) {
            return;
        }
        if let (Some(backend), Some(a)) = (
            self.backend.clone(),
            self.settings.accounts.iter().find(|a| a.id == id),
        ) {
            let context = ctx.clone();
            let wake = Arc::new(move || context.request_repaint());
            let w = Worker::start(
                id.into(),
                self.dir.join("accounts").join(id),
                backend,
                self.settings.interval,
                !a.enabled,
                self.settings.ephemeral_login,
                self.events_tx.clone(),
                wake,
            );
            self.workers.insert(id.into(), w);
        }
    }
    fn send(&mut self, id: &str, cmd: Command) {
        if self.demo {
            self.demo_command(id, cmd);
            return;
        }
        if let Some(worker) = self.workers.get(id) {
            if matches!(cmd, Command::Reset) {
                if let Some(s) = self.states.get_mut(id) {
                    s.status = Status::Resetting;
                    s.message = "正在核对最早到期卡片".into();
                }
            }
            worker.send(cmd);
        } else {
            self.notice = "官方组件尚未准备好，请在设置中检查".into();
        }
    }
    fn demo_command(&mut self, id: &str, cmd: Command) {
        if let Some(s) = self.states.get_mut(id) {
            match cmd {
                Command::Reset => {
                    if let Some(q) = &mut s.quota {
                        match q.choose_card(now()).map(|c| c.id.clone()) {
                            Ok(card) => {
                                q.rate_limits.primary.as_mut().unwrap().used_percent = 0.0;
                                let cards = q.rate_limit_reset_credits.as_mut().unwrap();
                                cards.available_count -= 1;
                                if let Some(rows) = &mut cards.credits {
                                    rows.retain(|c| c.id != card);
                                }
                                s.message = "模拟成功：已优先使用 2 小时后到期的卡".into();
                                q.fetched_at = now();
                            }
                            Err(e) => s.message = e,
                        }
                    }
                }
                Command::Pause(p) => {
                    s.status = if p { Status::Paused } else { Status::Ready };
                }
                Command::Refresh => {
                    if let Some(q) = &mut s.quota {
                        q.fetched_at = now();
                    }
                    s.status = Status::Ready;
                }
                Command::Logout => {
                    self.states.remove(id);
                    self.settings.accounts.retain(|a| a.id != id);
                    self.page = Page::Accounts;
                    self.dirty = true;
                }
                _ => {
                    s.message = "演示模式不连接真实账号".into();
                }
            }
        }
    }
    fn refresh_all(&mut self) {
        let ids: Vec<_> = self
            .settings
            .accounts
            .iter()
            .filter(|a| a.enabled)
            .map(|a| a.id.clone())
            .collect();
        for id in ids {
            self.send(&id, Command::Refresh);
        }
    }
    fn add(&mut self, ctx: &egui::Context) {
        if self
            .states
            .values()
            .any(|s| s.status == Status::WaitingLogin)
        {
            self.notice = "请先完成或取消正在进行的登录".into();
            return;
        }
        let n = self.settings.accounts.len() + 1;
        let a = AccountConfig {
            alias: format!("账号 {n}"),
            ..Default::default()
        };
        let id = a.id.clone();
        self.states.insert(
            id.clone(),
            if self.demo {
                engine::demo_state(&id, n - 1)
            } else {
                ViewState::new(id.clone(), None)
            },
        );
        self.settings.accounts.push(a);
        self.save();
        self.spawn(&id, ctx);
        self.page = Page::Detail(id.clone());
        if self.demo {
            return;
        }
        if self.backend.is_some() {
            self.send(&id, Command::Login { device: false });
        } else {
            self.need_login = Some(id);
        }
    }
    fn events(&mut self, ctx: &egui::Context) {
        if self.demo
            && self.last_demo_refresh.elapsed() >= Duration::from_secs(self.settings.interval)
        {
            for s in self.states.values_mut() {
                if s.status == Status::Ready {
                    if let Some(q) = &mut s.quota {
                        q.fetched_at = now();
                    }
                }
            }
            self.last_demo_refresh = Instant::now();
        }
        while let Ok(Boot::Ready(result)) = self.boot_rx.try_recv() {
            match result {
                Ok(b) => {
                    self.backend = Some(b);
                    self.notice.clear();
                    let ids: Vec<_> = self
                        .settings
                        .accounts
                        .iter()
                        .map(|a| a.id.clone())
                        .collect();
                    for id in ids {
                        self.spawn(&id, ctx);
                    }
                    if let Some(id) = self.need_login.take() {
                        self.send(&id, Command::Login { device: false });
                    }
                }
                Err(e) => self.notice = e,
            }
        }
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                Event::State(mut s) => {
                    if !self.settings.accounts.iter().any(|a| a.id == s.id) {
                        continue;
                    }
                    if s.quota.is_none() && s.status != Status::LoggedOut {
                        if let Some(old) = self.states.get(&s.id) {
                            s.quota = old.quota.clone();
                        }
                    }
                    if let Some(a) = self.settings.accounts.iter_mut().find(|a| a.id == s.id) {
                        a.cached = s.quota.clone();
                        self.dirty = true;
                    }
                    self.states.insert(s.id.clone(), s);
                }
                Event::OpenLogin(url) => {
                    if runtime::trusted_auth_url(&url) && open::that(&url).is_err() {
                        self.notice = "无法自动打开浏览器，请在账号详情点击登录地址".into();
                    }
                }
                Event::Removed(id) => {
                    self.workers.remove(&id);
                    self.states.remove(&id);
                    self.settings.accounts.retain(|a| a.id != id);
                    self.page = Page::Accounts;
                    self.dirty = true;
                }
            }
        }
        while let Ok(action) = self.tray_rx.try_recv() {
            match action {
                TrayAction::Hide => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
                TrayAction::Show => {
                    self.page = Page::Monitor;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    if !self.settings.lock_position {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                }
                TrayAction::Settings => {
                    self.page = Page::Settings;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayAction::ToggleLock => {
                    if self.settings.lock_position {
                        // This is the only user action that clears the lock.
                        self.settings.lock_position = false;
                        platform::set_overlay_locked(false);
                        self.dirty = true;
                    } else {
                        self.lock_overlay();
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                TrayAction::Refresh => self.refresh_all(),
                TrayAction::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                TrayAction::Available(v) => {
                    self.tray_available = v;
                    // Never leave a click-through window without a recovery icon.
                    if !v && self.settings.lock_position {
                        self.settings.lock_position = false;
                        platform::set_overlay_locked(false);
                        self.dirty = true;
                    }
                }
            }
        }
    }
    fn overlay_interaction(&mut self, r: &egui::Response, account: Option<&str>) {
        if account.is_some()
            && (r.hovered()
                || r.ctx
                    .input(|i| i.pointer.interact_pos().is_some_and(|p| r.rect.contains(p))))
        {
            self.context_account = account.map(String::from);
        }
        // Start on the press frame: waiting for the drag threshold is too late on Windows.
        if !self.settings.lock_position
            && (r.is_pointer_button_down_on()
                || (account.is_some()
                    && r.ctx
                        .input(|i| i.pointer.interact_pos().is_some_and(|p| r.rect.contains(p)))))
            && r.ctx
                .input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
        {
            platform::start_drag(&r.ctx);
        }
    }
    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if self.page != Page::Monitor {
                if ui.small_button("‹ 返回").clicked() {
                    self.page = Page::Monitor;
                }
            }
            let label = if self.demo {
                "额度 · 演示"
            } else {
                "额度监听"
            };
            let resp = ui.add(
                egui::Label::new(RichText::new(label).strong().size(14.0)).sense(Sense::drag()),
            );
            if resp.is_pointer_button_down_on()
                && ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
                && !self.settings.lock_position
            {
                platform::start_drag(ctx);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text(if self.tray_available {
                        "隐藏到系统托盘"
                    } else {
                        "退出程序"
                    })
                    .clicked()
                {
                    if self.tray_available {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    } else {
                        self.quitting = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                if ui.small_button("设置").clicked() {
                    self.page = Page::Settings;
                }
                if self.page == Page::Monitor
                    && ui
                        .small_button(if self.settings.view == "bar" {
                            "环形"
                        } else {
                            "横条"
                        })
                        .clicked()
                {
                    self.settings.view = if self.settings.view == "bar" {
                        "ring"
                    } else {
                        "bar"
                    }
                    .into();
                    self.dirty = true;
                }
            });
        });
        ui.add_space(4.0);
        ui.separator();
    }
    fn paint_text(ui: &egui::Ui, pos: egui::Pos2, galley: Arc<egui::Galley>) {
        let color = ui.visuals().override_text_color.unwrap_or(Color32::WHITE);
        ui.painter()
            .galley_with_override_text_color(pos, galley, color);
    }
    fn overlay_text(
        ui: &mut egui::Ui,
        text: String,
        font_size: f32,
        max_width: f32,
    ) -> egui::Response {
        let mut job = egui::text::LayoutJob::simple_singleline(
            text,
            FontId::proportional(font_size),
            Color32::WHITE,
        );
        job.wrap.max_width = max_width;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click_and_drag());
        Self::paint_text(ui, rect.min, galley);
        response
    }
    fn progress(ui: &mut egui::Ui, p: Option<f64>, ring: bool, stale: bool) -> egui::Response {
        let width = if ring { 74.0 } else { ui.available_width() };
        let height = if ring { 74.0 } else { 8.0 };
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let track = if p == Some(0.0) && !stale {
            Color32::from_rgb(255, 48, 64)
        } else if ui.visuals().dark_mode {
            Color32::from_rgb(58, 66, 71)
        } else {
            Color32::from_rgb(221, 227, 226)
        };
        let fill = if stale || p.is_none() {
            Color32::from_rgb(136, 147, 151)
        } else {
            let c = quota_color(p.unwrap());
            Color32::from_rgb(c[0], c[1], c[2])
        };
        if ring {
            let center = rect.center();
            let radius = 31.0;
            painter.circle_stroke(center, radius, Stroke::new(7.0_f32, Color32::BLACK));
            painter.circle_stroke(center, radius, Stroke::new(5.0_f32, track));
            if let Some(p) = p {
                if p > 0.0 {
                    let count = ((p / 100.0) * 80.0).ceil().max(2.0) as usize;
                    let pts = (0..=count)
                        .map(|i| {
                            let a = -std::f32::consts::FRAC_PI_2
                                + (p as f32 / 100.0) * std::f32::consts::TAU * i as f32
                                    / count as f32;
                            center + Vec2::new(a.cos(), a.sin()) * radius
                        })
                        .collect();
                    painter.add(egui::Shape::line(pts, Stroke::new(5.0_f32, fill)));
                }
            }
            let galley = painter.layout_no_wrap(
                p.map(remaining_text).unwrap_or("—".into()),
                FontId::proportional(17.0),
                Color32::WHITE,
            );
            let pos = center - galley.size() * 0.5;
            Self::paint_text(ui, pos, galley);
        } else {
            painter.rect_filled(rect.expand(1.0), 5.0, Color32::BLACK);
            painter.rect_filled(rect, 4.0, track);
            if let Some(p) = p {
                if p > 0.0 {
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            rect.min,
                            Vec2::new((rect.width() * p as f32 / 100.0).max(2.0), height),
                        ),
                        4.0,
                        fill,
                    );
                }
            }
        }
        response
    }
    fn lock_overlay(&mut self) {
        if self.tray_available {
            platform::cancel_drag();
            self.settings.lock_position = true;
            platform::set_overlay_locked(true);
            self.page = Page::Monitor;
            self.context_open = false;
            self.dirty = true;
        }
    }

    fn can_reset(&self, id: &str) -> bool {
        self.states.get(id).is_some_and(|state| {
            state.status != Status::Resetting
                && (state.pending
                    || (state.status == Status::Ready
                        && state.quota.as_ref().is_some_and(|q| {
                            !q.stale(now(), self.settings.interval) && q.choose_card(now()).is_ok()
                        })))
        })
    }

    fn overlay_action(&mut self, action: platform::OverlayAction, ctx: &egui::Context) {
        self.context_open = false;
        match action {
            platform::OverlayAction::Reset => {
                if let Some(id) = self.context_account.clone().filter(|id| self.can_reset(id)) {
                    self.send(&id, Command::Reset);
                }
            }
            platform::OverlayAction::Lock => self.lock_overlay(),
            platform::OverlayAction::Settings => self.page = Page::Settings,
            platform::OverlayAction::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint();
    }

    fn heading(&self, account: &AccountConfig) -> (String, String) {
        let name = if self.demo {
            format!("{} · 演示", account.alias)
        } else {
            account.alias.clone()
        };
        let suffix = self
            .states
            .get(&account.id)
            .and_then(|state| state.quota.as_ref())
            .map(Quota::compact_suffix)
            .unwrap_or_default();
        (name, suffix)
    }

    fn measure_monitor(
        &mut self,
        ctx: &egui::Context,
        geometry: Option<&platform::DesktopGeometry>,
    ) {
        let ring = self.settings.view == "ring";
        let font = FontId::proportional(13.0);
        let mut header_height = ctx
            .fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap("Ag国".into(), font.clone(), Color32::WHITE)
                    .size()
                    .y
            })
            .max(ctx.style().spacing.interact_size.y);
        let mut width = if ring { 96.0_f32 } else { 208.0_f32 };
        let mut count = 0;
        for account in self.settings.accounts.iter().filter(|a| a.visible) {
            count += 1;
            let (name, suffix) = self.heading(account);
            let title = format!("{name}{suffix}");
            let galley =
                ctx.fonts_mut(|fonts| fonts.layout_no_wrap(title, font.clone(), Color32::WHITE));
            header_height = header_height.max(galley.size().y);
            let percentage = if ring { 0.0 } else { 48.0 };
            width = width.max(galley.size().x + percentage + 16.0);
        }
        let available = geometry
            .map(|g| g.work.size() / ctx.pixels_per_point())
            .unwrap_or(Vec2::new(1920.0, 1080.0));
        width = width
            .min(if ring { 300.0 } else { 380.0 })
            .min((available.x - 8.0).max(32.0));
        let height = if count == 0 {
            header_height + 12.0
        } else {
            header_height + 5.0 + if ring { 74.0 } else { 8.0 } + 12.0
        };
        self.card_size = Vec2::new(width.ceil(), height.ceil());
        let fit = ((available.x - 2.0) / (self.card_size.x + 6.0))
            .floor()
            .max(1.0) as usize;
        self.columns = match self.edge {
            platform::Edge::Top | platform::Edge::Bottom => count.max(1).min(fit),
            platform::Edge::Free if ring => count.clamp(1, 3).min(fit),
            _ => 1,
        };
        let rows = count.max(1).div_ceil(self.columns);
        self.monitor_size = Vec2::new(
            self.columns as f32 * (self.card_size.x + 6.0) + 2.0,
            rows as f32 * (self.card_size.y + 6.0) + 2.0,
        );
    }

    fn monitor(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        let accounts: Vec<_> = self
            .settings
            .accounts
            .iter()
            .filter(|a| a.visible)
            .cloned()
            .collect();
        if accounts.is_empty() {
            let rect = egui::Rect::from_min_size(ui.next_widget_position(), self.card_size);
            Self::backdrop(ui, rect, egui::Id::new("empty-monitor"));
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(8.0, 6.0))),
                |ui| {
                    let r =
                        Self::overlay_text(ui, "右键添加账号".into(), 13.0, ui.available_width());
                    self.overlay_interaction(&r, None);
                },
            );
        } else {
            let ring = self.settings.view == "ring";
            for row in accounts.chunks(self.columns) {
                ui.horizontal_top(|ui| {
                    for account in row {
                        ui.allocate_ui_with_layout(
                            self.card_size,
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_size(self.card_size);
                                ui.set_max_size(self.card_size);
                                let rect = egui::Rect::from_min_size(
                                    ui.next_widget_position(),
                                    self.card_size,
                                );
                                Self::backdrop(
                                    ui,
                                    rect,
                                    egui::Id::new(("account-backdrop", &account.id)),
                                );
                                let inner = rect.shrink2(Vec2::new(8.0, 6.0));
                                ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                                    ui.set_width(inner.width());
                                    ui.spacing_mut().item_spacing = Vec2::new(0.0, 5.0);
                                    if ring {
                                        ui.vertical_centered(|ui| {
                                            self.account_widget(ui, account, true)
                                        });
                                    } else {
                                        self.account_widget(ui, account, false);
                                    }
                                });
                                let r = ui.interact(
                                    rect,
                                    ui.id().with(("overlay-drag", &account.id)),
                                    Sense::click_and_drag(),
                                );
                                self.overlay_interaction(&r, Some(&account.id));
                            },
                        );
                    }
                });
            }
        }
    }

    fn backdrop(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id) {
        let (text, fill) = platform::overlay_appearance(ui.ctx(), rect, id);
        ui.visuals_mut().override_text_color = Some(text);
        if fill.a() > 0 {
            ui.painter().rect_filled(rect, 6.0, fill);
        }
    }

    fn account_widget(&mut self, ui: &mut egui::Ui, account: &AccountConfig, ring: bool) {
        let state = self.states.get(&account.id);
        let quota = state.and_then(|s| s.quota.as_ref());
        let p = quota
            .and_then(Quota::tightest)
            .map(|(_, window)| window.remaining());
        let stale = state.is_none_or(|s| s.status != Status::Ready)
            || quota.is_none_or(|q| q.stale(now(), self.settings.interval));
        let (name, suffix) = self.heading(account);
        let percentage = p.map(remaining_text).unwrap_or("—".into());
        let suffix_width = ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(suffix.clone(), FontId::proportional(13.0), Color32::WHITE)
                .size()
                .x
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let reserve = suffix_width + if ring { 0.0 } else { 48.0 };
            let name_width = (ui.available_width() - reserve).max(12.0);
            let _ = Self::overlay_text(ui, name, 13.0, name_width);
            if !suffix.is_empty() {
                let _ = Self::overlay_text(ui, suffix, 13.0, suffix_width);
            }
            if !ring {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let _ = Self::overlay_text(ui, percentage, 13.0, 48.0);
                });
            }
        });
        let _ = Self::progress(ui, p, ring, stale);
    }

    #[cfg(not(windows))]
    fn fallback_context_menu(&mut self, ctx: &egui::Context) {
        if !self.context_open {
            return;
        }
        let can_reset = self
            .context_account
            .as_deref()
            .is_some_and(|id| self.can_reset(id));
        let mut action = None;
        egui::Window::new("悬浮菜单")
            .title_bar(false)
            .resizable(false)
            .fixed_pos(egui::pos2(4.0, 4.0))
            .show(ctx, |ui| {
                if ui
                    .add_enabled(can_reset, egui::Button::new("使用重置卡"))
                    .clicked()
                {
                    action = Some(platform::OverlayAction::Reset);
                }
                if ui
                    .add_enabled(self.tray_available, egui::Button::new("锁定悬浮"))
                    .clicked()
                {
                    action = Some(platform::OverlayAction::Lock);
                }
                if ui.button("打开菜单").clicked() {
                    action = Some(platform::OverlayAction::Settings);
                }
                if ui.button("退出").clicked() {
                    action = Some(platform::OverlayAction::Quit);
                }
            });
        if let Some(action) = action {
            self.overlay_action(action, ctx);
        }
    }

    fn accounts(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(RichText::new("账号管理").size(18.0).strong());
        let mut moves = None;
        let mut changes = vec![];
        for (i, a) in self.settings.accounts.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}", i + 1));
                if ui.text_edit_singleline(&mut a.alias).changed() {
                    self.dirty = true;
                }
                if ui.small_button("详情").clicked() {
                    self.page = Page::Detail(a.id.clone());
                }
            });
            ui.horizontal(|ui| {
                if ui.checkbox(&mut a.visible, "显示").changed() {
                    self.dirty = true;
                }
                if ui.checkbox(&mut a.enabled, "监测").changed() {
                    changes.push((a.id.clone(), !a.enabled));
                    self.dirty = true;
                }
                if i > 0 && ui.small_button("上移").clicked() {
                    moves = Some(i);
                }
            });
            ui.separator();
        }
        if let Some(i) = moves {
            self.settings.accounts.swap(i, i - 1);
            self.dirty = true;
        }
        for (id, p) in changes {
            self.send(&id, Command::Pause(p));
        }
        if ui.button("＋ 添加 OpenAI 账号").clicked() {
            self.add(ctx);
        }
        ui.label(
            RichText::new("每个账号独立登录；请在官方页面核对所选账号。")
                .small()
                .weak(),
        );
    }
    fn detail(&mut self, ui: &mut egui::Ui, id: &str) {
        let Some(account) = self.settings.accounts.iter().find(|a| a.id == id).cloned() else {
            return;
        };
        let s = self
            .states
            .get(id)
            .cloned()
            .unwrap_or_else(|| ViewState::new(id.into(), None));
        ui.label(RichText::new(&account.alias).size(20.0).strong());
        if let Some(email) = &s.email {
            ui.label(email);
        }
        if let Some(plan) = &s.plan {
            ui.label(RichText::new(format!("方案：{plan}")).small().weak());
        }
        ui.label(&s.message);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    s.status != Status::WaitingLogin && s.status != Status::Resetting,
                    egui::Button::new("浏览器登录"),
                )
                .clicked()
            {
                self.send(id, Command::Login { device: false });
            }
            if ui
                .add_enabled(
                    s.status != Status::WaitingLogin && s.status != Status::Resetting,
                    egui::Button::new("设备码登录"),
                )
                .clicked()
            {
                self.send(id, Command::Login { device: true });
            }
            if ui.button("刷新").clicked() {
                self.send(id, Command::Refresh);
            }
        });
        if let Some(url) = &s.login_url {
            if ui.button("打开官方登录页面").clicked() {
                let _ = open::that(url);
            }
            if let Some(code) = &s.user_code {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(code).monospace().size(22.0));
                    if ui.button("复制设备码").clicked() {
                        ui.ctx().copy_text(code.clone());
                    }
                });
            }
            if ui.button("取消登录").clicked() {
                self.send(id, Command::CancelLogin);
            }
        }
        if let Some(q) = &s.quota {
            ui.separator();
            ui.label(RichText::new("剩余额度").strong());
            for (label, w) in q.windows() {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.label(remaining_text(w.remaining()));
                });
                Self::progress(
                    ui,
                    Some(w.remaining()),
                    false,
                    q.stale(now(), self.settings.interval),
                );
                ui.label(
                    RichText::new(format!("恢复时间：{}", time_label(w.resets_at)))
                        .small()
                        .weak(),
                );
            }
            for (_, bucket) in q.buckets() {
                if let Some(balance) = &bucket.credits {
                    ui.label(format!(
                        "额外点数：{}",
                        if balance.unlimited {
                            "不限额".into()
                        } else {
                            balance.balance.clone().unwrap_or("未知".into())
                        }
                    ));
                }
                if let Some(limit) = &bucket.individual_limit {
                    ui.label(format!(
                        "个人预算剩余：{}",
                        remaining_text(limit.remaining_percent)
                    ));
                }
            }
            ui.label(
                RichText::new(format!("上次同步：{}", time_label(Some(q.fetched_at))))
                    .small()
                    .weak(),
            );
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "重置卡：{} 张",
                    q.card_count()
                        .map(|n| n.to_string())
                        .unwrap_or("未知".into())
                ))
                .strong(),
            );
            if let Some(cards) = q
                .rate_limit_reset_credits
                .as_ref()
                .and_then(|r| r.credits.as_ref())
            {
                let mut sorted: Vec<_> = cards.iter().collect();
                sorted.sort_by_key(|c| (c.expires_at.unwrap_or(i64::MAX), c.granted_at));
                for (i, c) in sorted.iter().enumerate() {
                    ui.label(format!(
                        "卡片 {} · {}{}",
                        i + 1,
                        time_label(c.expires_at),
                        if c.expires_at.is_some_and(|t| t <= now()) {
                            "（已过期）"
                        } else {
                            ""
                        }
                    ));
                }
            }
            ui.label(
                RichText::new("点击使用时，自动选择最早到期的一张。")
                    .small()
                    .weak(),
            );
            let choice = q.choose_card(now());
            let eligible = s.status == Status::Ready
                && !q.stale(now(), self.settings.interval)
                && choice.is_ok();
            if ui
                .add_enabled(
                    (eligible || s.pending) && s.status != Status::Resetting,
                    egui::Button::new(if s.pending {
                        "核对上次重置（不会重复扣卡）"
                    } else {
                        "使用 1 张最早到期卡"
                    }),
                )
                .clicked()
            {
                self.send(id, Command::Reset);
            }
            if let Err(reason) = choice {
                ui.label(RichText::new(reason).small().weak());
            }
        }
        if self.demo && ui.button("模拟额度耗尽").clicked() {
            if let Some(s) = self.states.get_mut(id) {
                if let Some(q) = &mut s.quota {
                    q.rate_limits.primary.as_mut().unwrap().used_percent = 100.0;
                    q.fetched_at = now();
                }
            }
        }
        ui.add_space(10.0);
        ui.separator();
        if ui
            .add_enabled(
                !s.pending && s.status != Status::Resetting,
                egui::Button::new("退出登录并移除此账号"),
            )
            .clicked()
        {
            self.send(id, Command::Logout);
        }
    }
    fn settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(RichText::new("设置").size(18.0).strong());
        if let Some(id) = self.context_account.clone() {
            if ui.button("此账号详情").clicked() {
                self.page = Page::Detail(id);
            }
        }
        if ui.button("刷新全部").clicked() {
            self.refresh_all();
        }
        if ui.button("添加账号").clicked() {
            self.add(ctx);
        }

        let before = self.settings.clone();
        ui.horizontal(|ui| {
            ui.label("显示方式");
            ui.selectable_value(&mut self.settings.view, "bar".into(), "横条");
            ui.selectable_value(&mut self.settings.view, "ring".into(), "圆环");
        });
        ui.horizontal(|ui| {
            ui.label("刷新间隔");
            ui.selectable_value(&mut self.settings.interval, 10, "10 秒");
            ui.selectable_value(&mut self.settings.interval, 30, "30 秒");
        });
        ui.add(egui::Slider::new(&mut self.settings.opacity, 0.45..=1.0).text("设置面板不透明度"));
        ui.add(egui::Slider::new(&mut self.settings.scale, 0.8..=1.8).text("界面缩放"));
        ui.checkbox(&mut self.settings.always_on_top, "始终置顶");
        ui.checkbox(&mut self.settings.light_theme, "浅色外观");
        if ui
            .add_enabled(
                !self.demo,
                egui::Checkbox::new(&mut self.settings.autostart, "开机启动"),
            )
            .changed()
        {
            match std::env::current_exe()
                .map_err(|_| "无法定位程序".to_string())
                .and_then(|exe| platform::autostart(self.settings.autostart, &exe))
            {
                Ok(()) => {}
                Err(e) => {
                    self.settings.autostart = before.autostart;
                    self.notice = e;
                }
            }
        }
        ui.separator();
        ui.label("登录保存方式");
        ui.checkbox(
            &mut self.settings.ephemeral_login,
            "仅本次会话（适用于系统凭据库不可用）",
        );
        ui.label(
            RichText::new("默认由官方组件将登录信息保存到系统凭据库。")
                .small()
                .weak(),
        );
        ui.label("官方组件路径（完整版本可留空）");
        ui.text_edit_singleline(&mut self.settings.codex_path);
        if ui.button("重新加载组件与登录设置").clicked() {
            self.workers.clear();
            self.backend = None;
            let root = self.dir.clone();
            let path = self.settings.codex_path.clone();
            let (tx, rx) = mpsc::channel();
            self.boot_rx = rx;
            let c = ctx.clone();
            self.notice = "正在准备官方组件…".into();
            std::thread::spawn(move || {
                let _ = tx.send(Boot::Ready(runtime::resolve(&root, &path)));
                c.request_repaint();
            });
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("管理账号").clicked() {
                self.page = Page::Accounts;
            }
            if ui.button("重置窗口位置").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    100.0, 100.0,
                )));
                self.settings.position = None;
            }
            if ui.button("打开数据目录").clicked() {
                let _ = open::that(&self.dir);
            }
        });
        ui.label(
            RichText::new(format!(
                "QuotaMonitori {} · 官方组件 {}",
                env!("CARGO_PKG_VERSION"),
                runtime::RUNTIME_VERSION
            ))
            .small()
            .weak(),
        );
        if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() {
            ui.label(
                RichText::new("Wayland 的置顶与窗口位置由桌面环境控制。")
                    .small()
                    .weak(),
            );
        }
        if !self.notice.is_empty() {
            ui.label(&self.notice);
        }
        if ui.button("退出程序").clicked() {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if serde_json::to_string(&before).ok() != serde_json::to_string(&self.settings).ok() {
            self.dirty = true;
        }
        if before.interval != self.settings.interval {
            for w in self.workers.values() {
                w.send(Command::Interval(self.settings.interval));
            }
        }
        if before.scale != self.settings.scale {
            ctx.set_zoom_factor(self.settings.scale);
        }
        if before.light_theme != self.settings.light_theme {
            self.apply_style(ctx);
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        platform::prepare_overlay();
        platform::update_drag(ctx);
        let mut geometry = platform::desktop_geometry(ctx);
        if self.was_monitor {
            self.monitor_rect = geometry.as_ref().map(|g| g.window);
        }
        self.events(ctx);
        let scale = ctx.pixels_per_point();
        if self.page == Page::Monitor {
            if let Some(g) = geometry
                .as_ref()
                .filter(|g| !g.dragging && self.was_monitor && !self.context_open)
            {
                self.edge = g.edge(scale);
            }
            self.measure_monitor(ctx, geometry.as_ref());
        }
        let open_context = self.page == Page::Monitor
            && !self.settings.lock_position
            && ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary));
        if open_context {
            self.context_account = None;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.page = Page::Monitor;
            self.context_open = false;
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.tray_available {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            if self.settings.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            },
        ));
        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            let pos = [rect.min.x, rect.min.y];
            if self.settings.position != Some(pos) {
                self.settings.position = Some(pos);
                self.dirty = true;
            }
        }
        let fill = if self.settings.light_theme {
            Color32::from_rgba_unmultiplied(246, 249, 248, (255.0 * self.settings.opacity) as u8)
        } else {
            Color32::from_rgba_unmultiplied(26, 32, 36, (255.0 * self.settings.opacity) as u8)
        };
        let frame = if self.page == Page::Monitor {
            egui::Frame::NONE.inner_margin(4)
        } else {
            egui::Frame::new()
                .fill(fill)
                .corner_radius(12)
                .inner_margin(12)
                .stroke(Stroke::new(
                    1.0_f32,
                    if self.settings.light_theme {
                        Color32::from_rgb(206, 218, 214)
                    } else {
                        Color32::from_rgb(61, 75, 75)
                    },
                ))
        };
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            if self.page != Page::Monitor {
                self.header(ui, ctx);
            }
            match self.page.clone() {
                Page::Monitor => {
                    egui::ScrollArea::vertical()
                        .max_height(620.0)
                        .show(ui, |ui| self.monitor(ui, ctx));
                }
                Page::Settings => {
                    egui::ScrollArea::vertical()
                        .max_height(620.0)
                        .show(ui, |ui| self.settings(ui, ctx));
                }
                Page::Accounts => {
                    egui::ScrollArea::vertical()
                        .max_height(560.0)
                        .show(ui, |ui| self.accounts(ui, ctx));
                }
                Page::Detail(id) => {
                    egui::ScrollArea::vertical()
                        .max_height(640.0)
                        .show(ui, |ui| self.detail(ui, &id));
                }
            }
        });
        if open_context {
            let can_reset = self
                .context_account
                .as_deref()
                .is_some_and(|id| self.can_reset(id));
            if let Some(action) = platform::overlay_menu(can_reset, self.tray_available) {
                self.overlay_action(action, ctx);
            }
            #[cfg(not(windows))]
            {
                self.context_open = true;
            }
            ctx.input_mut(|input| input.pointer = Default::default());
        }
        #[cfg(not(windows))]
        self.fallback_context_menu(ctx);
        platform::set_overlay_locked(self.settings.lock_position);
        ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(
            self.settings.lock_position && self.page == Page::Monitor,
        ));
        if self.page == Page::Monitor && !self.was_monitor {
            if let (Some(g), Some(rect)) = (geometry.as_mut(), self.monitor_rect) {
                g.window = rect;
            }
            self.last_size = Vec2::ZERO;
        }
        let width = if self.page == Page::Monitor {
            self.monitor_size.x
        } else {
            470.0
        };
        let height = match self.page {
            Page::Settings => 680.0,
            Page::Accounts => 460.0,
            Page::Detail(_) => 700.0,
            Page::Monitor => self.monitor_size.y,
        };
        let height = if self.context_open {
            height.max(160.0)
        } else {
            height
        };
        let available = geometry
            .as_ref()
            .map(|g| g.work.size() / scale)
            .unwrap_or(Vec2::new(1920.0, 1080.0));
        let size = Vec2::new(
            width.min(available.x),
            height.clamp(36.0, 700.0).min(available.y),
        );
        let size_changed = (size - self.last_size).length() > 1.0;
        let needs_recovery = geometry.as_ref().is_some_and(|g| g.needs_recovery());
        if (size_changed || needs_recovery) && geometry.as_ref().is_none_or(|g| !g.dragging) {
            platform::resize_overlay(ctx, size, self.edge, geometry.as_ref());
            self.last_size = size;
            ctx.request_repaint();
        }
        if self.dirty && self.last_save.elapsed() > Duration::from_secs(2) {
            self.save();
        }
        self.was_monitor = self.page == Page::Monitor;
        // Keep background contrast current without a continuous rendering loop.
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save();
        self.workers.clear();
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    fn fixture(ctx: &egui::Context, view: &str) -> App {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "cjk".into(),
            egui::FontData::from_static(include_bytes!("../assets/NotoSansSC-Subset.otf")).into(),
        );
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .push("cjk".into());
        ctx.set_fonts(fonts);
        let settings = Settings {
            view: view.into(),
            accounts: ["主账号", "备用账号", "工作账号"]
                .into_iter()
                .map(|alias| AccountConfig {
                    alias: alias.into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let states = settings
            .accounts
            .iter()
            .enumerate()
            .map(|(i, a)| (a.id.clone(), engine::demo_state(&a.id, i)))
            .collect();
        let (events_tx, events_rx) = mpsc::channel();
        let app = App {
            dir: PathBuf::new(),
            settings,
            demo: true,
            _lock: std::fs::File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap(),
            page: Page::Monitor,
            states,
            workers: HashMap::new(),
            backend: None,
            events_tx,
            events_rx,
            boot_rx: mpsc::channel().1,
            tray_rx: mpsc::channel().1,
            _tray: Tray::default(),
            tray_available: false,
            quitting: false,
            notice: String::new(),
            dirty: false,
            last_save: Instant::now(),
            last_demo_refresh: Instant::now(),
            last_size: Vec2::ZERO,
            need_login: None,
            context_account: None,
            edge: platform::Edge::Free,
            columns: 1,
            was_monitor: true,
            monitor_rect: None,
            card_size: Vec2::new(208.0, 43.0),
            monitor_size: Vec2::new(216.0, 160.0),
            context_open: false,
        };
        app.apply_style(ctx);
        app
    }

    #[test]
    fn painted_monitor_fits_padded_cards_without_text_outlines() {
        for view in ["bar", "ring"] {
            let ctx = egui::Context::default();
            let mut app = fixture(&ctx, view);
            let mut output = egui::FullOutput::default();
            for _ in 0..3 {
                output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            app.monitor_size,
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        app.measure_monitor(ctx, None);
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE.inner_margin(4))
                            .show(ctx, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(620.0)
                                    .show(ui, |ui| app.monitor(ui, ctx));
                            });
                    },
                );
            }
            let cards: Vec<_> = (0..3)
                .map(|i| {
                    egui::Rect::from_min_size(
                        egui::pos2(
                            4.0 + (i % app.columns) as f32 * (app.card_size.x + 6.0),
                            4.0 + (i / app.columns) as f32 * (app.card_size.y + 6.0),
                        ),
                        app.card_size,
                    )
                })
                .collect();
            let texts: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                texts.len(),
                8,
                "{view}: each title, nonzero suffix and percentage must be painted once: {:?}",
                texts
                    .iter()
                    .map(|t| (&t.galley.job.text, t.pos))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                texts
                    .iter()
                    .filter(|t| t.galley.job.text.contains("重置卡"))
                    .count(),
                2
            );
            assert_eq!(
                texts
                    .iter()
                    .filter(|t| t.galley.job.text.contains("积分"))
                    .count(),
                1
            );
            for text in texts {
                let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                assert!(
                    cards.iter().any(|card| card
                        .shrink2(Vec2::new(8.0, 6.0))
                        .expand(0.5)
                        .contains_rect(bounds)),
                    "{view}: text outside padded cards: {:?} at {bounds:?}; cards: {cards:?}",
                    text.galley.job.text
                );
            }
            if view == "bar" {
                let tracks: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if (rect.rect.height() - 8.0).abs() < 0.1
                                && (rect.rect.width() - app.card_size.x + 16.0).abs() < 0.1 =>
                        {
                            Some(rect.rect)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(tracks.len(), 3);
                for (track, card) in tracks.iter().zip(cards) {
                    assert!(
                        (card.bottom() - track.bottom() - 6.0).abs() < 0.5,
                        "unexpected bottom padding: {track:?} in {card:?}"
                    );
                }
            }
        }
    }
}
