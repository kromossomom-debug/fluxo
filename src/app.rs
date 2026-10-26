//! Interface do Fluxo: barra lateral, início (busca/link), downloads, biblioteca e configurações.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use egui::{
    vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Sense, Stroke,
    TextureHandle, Ui,
};

use crate::engine::{self, ChildSlot, Event, MediaInfo, Progress, Quality};
use crate::store::{self, HistoryItem, Settings};
use crate::theme::{self, Palette, ACCENT_A, ACCENT_B, DANGER, SUCCESS, WARNING};

const SITES: &[(&str, &str, Color32)] = &[
    ("YouTube", "https://www.youtube.com", Color32::from_rgb(0xFF, 0x2D, 0x2D)),
    ("Instagram", "https://www.instagram.com", Color32::from_rgb(0xE1, 0x30, 0x6C)),
    ("TikTok", "https://www.tiktok.com", Color32::from_rgb(0x25, 0xF4, 0xEE)),
    ("Facebook", "https://www.facebook.com/watch", Color32::from_rgb(0x18, 0x77, 0xF2)),
    ("X", "https://x.com", Color32::from_rgb(0x9A, 0xA0, 0xB0)),
    ("Vimeo", "https://vimeo.com", Color32::from_rgb(0x1A, 0xB7, 0xEA)),
    ("SoundCloud", "https://soundcloud.com", Color32::from_rgb(0xFF, 0x77, 0x00)),
    ("Dailymotion", "https://www.dailymotion.com", Color32::from_rgb(0x00, 0xAA, 0xFF)),
    ("Twitch", "https://www.twitch.tv", Color32::from_rgb(0x91, 0x46, 0xFF)),
    ("Reddit", "https://www.reddit.com", Color32::from_rgb(0xFF, 0x45, 0x00)),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Downloads,
    Library,
    Settings,
}

enum Msg {
    Search(Result<Vec<MediaInfo>, String>),
    Info(u64, Result<MediaInfo, String>),
    Task(u64, Event),
    Tools(Option<String>, bool),
    ToolProgress(f32),
    ToolDone(Result<String, String>),
}

#[derive(Clone, PartialEq)]
enum TaskState {
    Queued,
    Running,
    Done(PathBuf),
    Failed(String),
    Canceled,
}

struct Task {
    id: u64,
    media: MediaInfo,
    quality: Quality,
    state: TaskState,
    progress: Progress,
    child: ChildSlot,
}

/// Janela de detalhes de um vídeo, com a escolha de qualidade.
struct Picker {
    request: u64,
    media: MediaInfo,
    loading: bool,
    error: Option<String>,
    quality: Quality,
}

struct Toast {
    text: String,
    color: Color32,
    until: f64,
}

pub struct FluxoApp {
    settings: Settings,
    history: Vec<HistoryItem>,
    page: Page,
    query: String,
    results: Vec<MediaInfo>,
    results_title: String,
    searching: bool,
    search_error: Option<String>,
    picker: Option<Picker>,
    tasks: Vec<Task>,
    next_id: u64,
    ytdlp: Option<String>,
    ffmpeg: bool,
    tools_checked: bool,
    installing: Option<(String, f32)>,
    toast: Option<Toast>,
    library_filter: String,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    gradient: TextureHandle,
    applied_dark: Option<bool>,
    /// Busca/link vindo da linha de comando, executado quando o motor estiver pronto.
    pending_submit: bool,
}

impl FluxoApp {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let app = Self {
            settings: store::load_settings(),
            history: store::load_history(),
            page: Page::Home,
            query: String::new(),
            results: Vec::new(),
            results_title: String::new(),
            searching: false,
            search_error: None,
            picker: None,
            tasks: Vec::new(),
            next_id: 1,
            ytdlp: None,
            ffmpeg: false,
            tools_checked: false,
            installing: None,
            toast: None,
            library_filter: String::new(),
            tx,
            rx,
            ctx: ctx.clone(),
            gradient: theme::gradient_texture(ctx),
            applied_dark: None,
            pending_submit: false,
        };
        let mut app = app;
        // Argumentos: `fluxo [--page downloads|library|settings] [URL ou termo de busca]`
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--page" {
                app.page = match args.next().as_deref() {
                    Some("downloads") => Page::Downloads,
                    Some("library") => Page::Library,
                    Some("settings") => Page::Settings,
                    _ => Page::Home,
                };
            } else {
                app.query = arg;
                app.pending_submit = true;
            }
        }
        app.check_tools();
        app
    }

    fn pal(&self) -> Palette {
        Palette::get(self.settings.dark)
    }

    fn send_from_thread(&self) -> impl Fn(Msg) + Send + Clone + 'static {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        move |m| {
            let _ = tx.send(m);
            ctx.request_repaint();
        }
    }

    fn check_tools(&self) {
        let send = self.send_from_thread();
        std::thread::spawn(move || send(Msg::Tools(engine::ytdlp_version(), engine::ffmpeg_path().is_some())));
    }

    fn toast(&mut self, text: impl Into<String>, color: Color32) {
        let now = self.ctx.input(|i| i.time);
        self.toast = Some(Toast { text: text.into(), color, until: now + 3.5 });
    }

    // ---------------------------------------------------------------- ações

    fn submit_query(&mut self) {
        let q = self.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        if self.ytdlp.is_none() {
            self.toast("Instale o motor yt-dlp primeiro (Configurações)", WARNING);
            return;
        }
        if q.starts_with("http://") || q.starts_with("https://") || q.starts_with("www.") {
            let url = if q.starts_with("www.") { format!("https://{q}") } else { q };
            self.open_picker(MediaInfo { url, title: "Carregando…".into(), partial: true, ..Default::default() });
        } else {
            self.searching = true;
            self.search_error = None;
            self.results_title = format!("Resultados para “{q}”");
            let count = self.settings.search_results;
            let send = self.send_from_thread();
            std::thread::spawn(move || send(Msg::Search(engine::search(&q, count))));
        }
    }

    fn open_picker(&mut self, media: MediaInfo) {
        let request = self.next_id;
        self.next_id += 1;
        let loading = media.partial;
        if loading {
            let url = media.url.clone();
            let send = self.send_from_thread();
            std::thread::spawn(move || send(Msg::Info(request, engine::fetch_info(&url))));
        }
        let quality = self.pick_default_quality(&media);
        self.picker = Some(Picker { request, media, loading, error: None, quality });
    }

    fn pick_default_quality(&self, media: &MediaInfo) -> Quality {
        match self.settings.default_quality {
            Quality::Video(max) => {
                let h = media.heights.iter().copied().find(|h| *h <= max).unwrap_or(max);
                Quality::Video(h)
            }
            q => q,
        }
    }

    fn enqueue(&mut self, media: MediaInfo, quality: Quality) {
        let id = self.next_id;
        self.next_id += 1;
        self.toast(format!("Adicionado aos downloads: {}", media.title), SUCCESS);
        self.tasks.insert(
            0,
            Task {
                id,
                media,
                quality,
                state: TaskState::Queued,
                progress: Progress::default(),
                child: Arc::new(Mutex::new(None)),
            },
        );
    }

    fn schedule(&mut self) {
        let running = self.tasks.iter().filter(|t| t.state == TaskState::Running).count();
        let free = self.settings.max_parallel.saturating_sub(running);
        let dir = self.settings.download_dir.clone();
        let _ = std::fs::create_dir_all(&dir);
        let send = self.send_from_thread();
        for task in self.tasks.iter_mut().rev().filter(|t| t.state == TaskState::Queued).take(free) {
            task.state = TaskState::Running;
            task.progress = Progress { stage: "Conectando…".into(), ..Default::default() };
            let (id, url, quality, child, dir, send) =
                (task.id, task.media.url.clone(), task.quality, task.child.clone(), dir.clone(), send.clone());
            std::thread::spawn(move || {
                engine::download(&url, quality, &dir, child, move |ev| send(Msg::Task(id, ev)));
            });
        }
    }

    fn install_tool(&mut self, ffmpeg: bool) {
        let name = if ffmpeg { "ffmpeg" } else { "yt-dlp" };
        self.installing = Some((format!("Baixando {name}…"), 0.0));
        let send = self.send_from_thread();
        std::thread::spawn(move || {
            let progress = {
                let send = send.clone();
                move |f| send(Msg::ToolProgress(f))
            };
            let result = if ffmpeg { engine::install_ffmpeg(progress) } else { engine::install_ytdlp(progress) };
            send(Msg::ToolDone(result.map(|_| format!("{name} instalado com sucesso"))));
        });
    }

    fn update_ytdlp(&mut self) {
        self.installing = Some(("Atualizando yt-dlp…".into(), -1.0));
        let send = self.send_from_thread();
        std::thread::spawn(move || send(Msg::ToolDone(engine::update_ytdlp())));
    }

    fn handle_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Search(res) => {
                    self.searching = false;
                    match res {
                        Ok(list) => {
                            if list.is_empty() {
                                self.search_error = Some("Nenhum resultado encontrado.".into());
                            }
                            self.results = list;
                        }
                        Err(e) => self.search_error = Some(e),
                    }
                }
                Msg::Info(request, res) => {
                    let default = res.as_ref().ok().map(|m| self.pick_default_quality(m));
                    if let Some(p) = self.picker.as_mut().filter(|p| p.request == request) {
                        p.loading = false;
                        match res {
                            Ok(m) => {
                                p.media = m;
                                if !p.quality.is_audio() {
                                    p.quality = default.unwrap_or(p.quality);
                                }
                            }
                            Err(e) => p.error = Some(e),
                        }
                    }
                }
                Msg::Task(id, ev) => {
                    let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else { continue };
                    if task.state == TaskState::Canceled {
                        continue;
                    }
                    match ev {
                        Event::Progress(p) => task.progress = p,
                        Event::Done(path) => {
                            task.state = TaskState::Done(path.clone());
                            task.progress.fraction = 1.0;
                            let item = HistoryItem {
                                title: task.media.title.clone(),
                                url: task.media.url.clone(),
                                thumbnail: task.media.thumbnail.clone(),
                                uploader: task.media.uploader.clone(),
                                path,
                                quality: task.quality,
                                size: task.progress.total.clone(),
                                finished_at: store::now(),
                            };
                            let title = item.title.clone();
                            self.history.insert(0, item);
                            store::save_history(&self.history);
                            self.toast(format!("Concluído: {title}"), SUCCESS);
                        }
                        Event::Failed(e) => task.state = TaskState::Failed(e),
                    }
                }
                Msg::Tools(ytdlp, ffmpeg) => {
                    self.ytdlp = ytdlp;
                    self.ffmpeg = ffmpeg;
                    self.tools_checked = true;
                }
                Msg::ToolProgress(f) => {
                    if let Some(i) = self.installing.as_mut() {
                        i.1 = f;
                    }
                }
                Msg::ToolDone(res) => {
                    self.installing = None;
                    match res {
                        Ok(m) => self.toast(if m.is_empty() { "Pronto".into() } else { m }, SUCCESS),
                        Err(e) => self.toast(e, DANGER),
                    }
                    self.check_tools();
                }
            }
        }
    }

    // -------------------------------------------------------------- widgets

    fn card(&self) -> Frame {
        let p = self.pal();
        Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(14)
            .inner_margin(Margin::same(16))
    }

    /// Linha de `n` cards de mesma largura que respeita a largura disponível.
    fn card_row(&self, ui: &mut Ui, n: usize, min_height: f32, mut add: impl FnMut(usize, &mut Ui)) {
        let gap = 14.0;
        let outer = (ui.available_width() - gap * (n as f32 - 1.0)) / n as f32;
        let inner = (outer - 34.0).max(40.0); // margens internas (2×16) + borda
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for i in 0..n {
                ui.allocate_ui_with_layout(vec2(outer, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_max_width(outer);
                    self.card().show(ui, |ui| {
                        ui.set_width(inner);
                        ui.set_min_height(min_height);
                        add(i, ui);
                    });
                });
            }
        });
    }

    /// Botão principal com gradiente.
    fn primary_button(&self, ui: &mut Ui, text: &str, size: egui::Vec2) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            theme::gradient_rect(painter, &self.gradient, rect, CornerRadius::same(10));
            if resp.hovered() {
                painter.rect_filled(rect, 10, Color32::from_white_alpha(28));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            painter.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(15.5), Color32::WHITE);
        }
        resp
    }

    fn chip(ui: &mut Ui, text: &str, fill: Color32, color: Color32) {
        Frame::new()
            .fill(fill)
            .corner_radius(20)
            .inner_margin(Margin::symmetric(9, 3))
            .show(ui, |ui| ui.label(RichText::new(text).size(12.0).color(color)));
    }

    fn progress_bar(&self, ui: &mut Ui, fraction: f32, color: Option<Color32>) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 8.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 4, self.pal().border);
        let mut fill = rect;
        fill.set_width((rect.width() * fraction.clamp(0.0, 1.0)).max(8.0));
        match color {
            Some(c) => {
                painter.rect_filled(fill, 4, c);
            }
            None => theme::gradient_rect(painter, &self.gradient, fill, CornerRadius::same(4)),
        }
    }

    fn thumbnail(ui: &mut Ui, url: &str, size: egui::Vec2, radius: u8, fallback: Color32) -> egui::Response {
        if url.is_empty() {
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
            ui.painter().rect_filled(rect, radius, fallback);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "🎬", FontId::proportional(size.y * 0.35), Color32::from_white_alpha(120));
            resp
        } else {
            ui.add(
                egui::Image::new(url)
                    .fit_to_exact_size(size)
                    .maintain_aspect_ratio(false)
                    .corner_radius(radius)
                    .sense(Sense::click()),
            )
        }
    }

    // --------------------------------------------------------------- páginas

    fn sidebar(&mut self, ui: &mut Ui) {
        let p = self.pal();
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let (rect, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
            theme::gradient_rect(ui.painter(), &self.gradient, rect, CornerRadius::same(12));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "⬇", FontId::proportional(21.0), Color32::WHITE);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("Fluxo").size(21.0).strong());
                ui.label(RichText::new("Downloader de vídeos").size(11.5).color(p.muted));
            });
        });
        ui.add_space(26.0);

        let active = self.tasks.iter().filter(|t| matches!(t.state, TaskState::Running | TaskState::Queued)).count();
        let items = [
            (Page::Home, "🏠", "Início", 0),
            (Page::Downloads, "⬇", "Downloads", active),
            (Page::Library, "📚", "Biblioteca", 0),
            (Page::Settings, "⚙", "Configurações", 0),
        ];
        for (page, icon, label, badge) in items {
            let selected = self.page == page;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
            let painter = ui.painter();
            if selected {
                painter.rect_filled(rect, 10, ACCENT_A.gamma_multiply(if self.settings.dark { 0.16 } else { 0.10 }));
                let bar = egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 10.0), vec2(3.0, 22.0));
                theme::gradient_rect(painter, &self.gradient, bar, CornerRadius::same(2));
            } else if resp.hovered() {
                painter.rect_filled(rect, 10, p.surface_hover);
            }
            let color = if selected { ACCENT_A } else { p.muted };
            painter.text(rect.left_center() + vec2(18.0, 0.0), Align2::LEFT_CENTER, icon, FontId::proportional(17.0), color);
            painter.text(
                rect.left_center() + vec2(46.0, 0.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(15.0),
                if selected { p.text } else { p.muted },
            );
            if badge > 0 {
                let c = rect.right_center() - vec2(20.0, 0.0);
                painter.circle_filled(c, 10.0, ACCENT_A);
                painter.text(c, Align2::CENTER_CENTER, badge.to_string(), FontId::proportional(11.5), Color32::WHITE);
            }
            if resp.clicked() {
                self.page = page;
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(16.0);
            let (dot, text) = match (&self.ytdlp, self.tools_checked) {
                (Some(v), _) => (SUCCESS, format!("Motor pronto · {v}")),
                (None, false) => (p.muted, "Verificando motor…".to_string()),
                (None, true) => (WARNING, "Motor não instalado".to_string()),
            };
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, dot);
                ui.label(RichText::new(text).size(12.0).color(p.muted));
            });
            let label = if self.settings.dark { "☀  Tema claro" } else { "🌙  Tema escuro" };
            if ui.add(egui::Button::new(label).frame(false)).clicked() {
                self.settings.dark = !self.settings.dark;
                store::save_settings(&self.settings);
            }
        });
    }

    fn home(&mut self, ui: &mut Ui) {
        let p = self.pal();
        // Banner com busca
        let hero_h = 210.0;
        let (hero, _) = ui.allocate_exact_size(vec2(ui.available_width(), hero_h), Sense::hover());
        theme::gradient_rect(ui.painter(), &self.gradient, hero, CornerRadius::same(20));
        // Círculos decorativos
        let painter = ui.painter().with_clip_rect(hero);
        painter.circle_filled(hero.right_top() + vec2(-60.0, 20.0), 110.0, Color32::from_white_alpha(22));
        painter.circle_filled(hero.right_bottom() + vec2(-200.0, 40.0), 70.0, Color32::from_white_alpha(16));

        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(hero.shrink2(vec2(32.0, 28.0))));
        inner.label(RichText::new("Baixe vídeos e músicas").size(30.0).strong().color(Color32::WHITE));
        inner.label(
            RichText::new("Cole um link do YouTube, Instagram, TikTok e de mais de 1000 sites — ou pesquise direto aqui.")
                .size(15.0)
                .color(Color32::from_white_alpha(225)),
        );
        inner.add_space(14.0);
        let mut submit = false;
        inner.horizontal(|ui| {
            let btn_w = 130.0;
            Frame::new()
                .fill(if self.settings.dark { Color32::from_rgb(0x12, 0x14, 0x1B) } else { Color32::WHITE })
                .corner_radius(12)
                .inner_margin(Margin::symmetric(14, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("🔍").size(17.0).color(p.muted));
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.query)
                                .hint_text("Pesquisar ou colar link…")
                                .frame(Frame::NONE)
                                .font(FontId::proportional(17.0))
                                .desired_width(ui.available_width() - btn_w - 40.0)
                                .margin(Margin::symmetric(0, 6)),
                        );
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            submit = true;
                        }
                    });
                });
            let is_url = self.query.trim_start().starts_with("http") || self.query.trim_start().starts_with("www.");
            let text = if is_url { "⬇  Baixar" } else { "Buscar" };
            let (rect, resp) = ui.allocate_exact_size(vec2(btn_w, 44.0), Sense::click());
            ui.painter().rect_filled(rect, 12, if resp.hovered() { Color32::from_white_alpha(250) } else { Color32::WHITE });
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(16.0), ACCENT_A);
            if resp.clicked() {
                submit = true;
            }
        });
        if submit {
            self.submit_query();
        }

        ui.add_space(14.0);
        if self.tools_checked && self.ytdlp.is_none() {
            self.card().fill(WARNING.gamma_multiply(0.12)).stroke(Stroke::new(1.0, WARNING.gamma_multiply(0.5))).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").size(22.0).color(WARNING));
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - 200.0).max(200.0));
                        ui.label(RichText::new("Falta o motor de downloads").strong());
                        ui.label(RichText::new("O Fluxo usa o yt-dlp (código aberto) para baixar. Instale com um clique.").color(p.muted));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.installing.is_none() && self.primary_button(ui, "Instalar yt-dlp", vec2(150.0, 38.0)).clicked() {
                            self.install_tool(false);
                        }
                        if let Some((t, f)) = &self.installing {
                            ui.label(format!("{t} {:.0}%", f.max(0.0) * 100.0));
                            ui.spinner();
                        }
                    });
                });
            });
            ui.add_space(10.0);
        }

        // Atalhos de sites
        ui.label(RichText::new("Sites populares").size(17.0).strong());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for (name, url, color) in SITES {
                let galley = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(14.0), p.text);
                let size = vec2(galley.size().x + 44.0, 36.0);
                let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                let painter = ui.painter();
                painter.rect(
                    rect,
                    18,
                    if resp.hovered() { p.surface_hover } else { p.surface },
                    Stroke::new(1.0, if resp.hovered() { *color } else { p.border }),
                    egui::StrokeKind::Inside,
                );
                painter.circle_filled(rect.left_center() + vec2(18.0, 0.0), 6.0, *color);
                painter.galley(rect.left_center() + vec2(32.0, -galley.size().y / 2.0), galley, p.text);
                if resp.clicked() {
                    let _ = open::that(url);
                }
                resp.on_hover_text("Abrir no navegador — copie o link de um vídeo e cole aqui");
            }
        });
        ui.add_space(12.0);

        // Resultados
        if self.searching {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Buscando…").color(p.muted));
            });
        } else if let Some(err) = &self.search_error {
            ui.label(RichText::new(format!("⚠ {err}")).color(DANGER));
        }

        if self.results.is_empty() && !self.searching {
            self.empty_home(ui);
            return;
        }
        if !self.results.is_empty() {
            ui.label(RichText::new(&self.results_title).size(17.0).strong());
            ui.add_space(4.0);
        }
        let mut open: Option<MediaInfo> = None;
        let mut quick: Option<MediaInfo> = None;
        let results = &self.results;
        grid(ui, results.len(), 200.0, 16.0, |ui, i, card_w| {
            let media = &results[i];
            let thumb = vec2(card_w, card_w * 9.0 / 16.0);
            {
                let resp = {
                    {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        let img = Self::thumbnail(ui, &media.thumbnail, thumb, 12, p.surface_hover);
                        if let Some(d) = media.duration {
                            let text = fmt_duration(d);
                            let galley = ui.painter().layout_no_wrap(text, FontId::proportional(12.0), Color32::WHITE);
                            let r = egui::Rect::from_min_size(
                                img.rect.right_bottom() - galley.size() - vec2(16.0, 12.0),
                                galley.size() + vec2(10.0, 4.0),
                            );
                            ui.painter().rect_filled(r, 6, Color32::from_black_alpha(190));
                            ui.painter().galley(r.min + vec2(5.0, 2.0), galley, Color32::WHITE);
                        }
                        // Botão de download rápido sobre a miniatura
                        let q_rect = egui::Rect::from_min_size(img.rect.right_top() + vec2(-44.0, 8.0), vec2(36.0, 36.0));
                        let q_resp = ui.interact(q_rect, ui.scope_id().with(("quick", &media.id)), Sense::click());
                        if img.hovered() || q_resp.hovered() {
                            ui.painter().rect_filled(img.rect, 12, Color32::from_black_alpha(60));
                            ui.painter().circle_filled(q_rect.center(), 18.0, if q_resp.hovered() { ACCENT_B } else { ACCENT_A });
                            ui.painter().text(q_rect.center(), Align2::CENTER_CENTER, "⬇", FontId::proportional(17.0), Color32::WHITE);
                            ui.painter().circle_filled(img.rect.center(), 24.0, Color32::from_black_alpha(140));
                            ui.painter().text(img.rect.center(), Align2::CENTER_CENTER, "▶", FontId::proportional(20.0), Color32::WHITE);
                        }
                        let title = ui.add(
                            egui::Label::new(RichText::new(&media.title).size(14.5).strong())
                                .wrap_mode(egui::TextWrapMode::Truncate)
                                .sense(Sense::click()),
                        );
                        let mut meta = media.uploader.clone();
                        if let Some(v) = media.views {
                            if !meta.is_empty() {
                                meta.push_str(" · ");
                            }
                            meta.push_str(&fmt_views(v));
                        }
                        ui.label(RichText::new(meta).size(12.5).color(p.muted));
                        (img.clicked() || title.clicked(), q_resp.clicked())
                    }
                };
                if resp.1 {
                    quick = Some(media.clone());
                } else if resp.0 {
                    open = Some(media.clone());
                }
            }
        });
        if let Some(m) = quick {
            // Download rápido usa a qualidade padrão das configurações.
            let q = self.settings.default_quality;
            self.enqueue(m, q);
        } else if let Some(m) = open {
            self.open_picker(m);
        }
    }

    fn empty_home(&mut self, ui: &mut Ui) {
        let p = self.pal();
        ui.add_space(6.0);
        let steps = [
            ("1", "Copie o link", "Em qualquer site de vídeo, copie o endereço do vídeo ou da música."),
            ("2", "Cole no Fluxo", "Cole na barra acima. Ou digite um termo para pesquisar no YouTube."),
            ("3", "Escolha e baixe", "Selecione a qualidade — de 360p a 4K, ou só o áudio em MP3."),
        ];
        let gradient = self.gradient.clone();
        self.card_row(ui, 3, 100.0, |i, ui| {
            let (n, title, text) = steps[i];
            let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
            theme::gradient_rect(ui.painter(), &gradient, r, CornerRadius::same(15));
            ui.painter().text(r.center(), Align2::CENTER_CENTER, n, FontId::proportional(15.0), Color32::WHITE);
            ui.add(egui::Label::new(RichText::new(title).size(16.0).strong()).truncate());
            // Texto com largura de quebra fixa: garante que o card não estoure a linha.
            let width = ui.max_rect().width();
            let galley = ui.painter().layout(text.to_string(), FontId::proportional(15.0), p.muted, width);
            let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
            ui.painter().galley(rect.min, galley, p.muted);
        });

        if !self.history.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("Baixados recentemente").size(17.0).strong());
            let items: Vec<HistoryItem> = self.history.iter().take(6).cloned().collect();
            egui::ScrollArea::horizontal().id_salt("recent").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for item in items {
                        ui.vertical(|ui| {
                            ui.set_width(180.0);
                            if Self::thumbnail(ui, &item.thumbnail, vec2(180.0, 101.0), 10, p.surface_hover).clicked() {
                                let _ = open::that(&item.path);
                            }
                            ui.add(egui::Label::new(RichText::new(&item.title).size(13.0)).truncate());
                        });
                    }
                });
            });
        }
    }

    fn picker_modal(&mut self, ctx: &egui::Context) {
        let Some(picker) = self.picker.as_mut() else { return };
        let p = Palette::get(self.settings.dark);
        let ffmpeg = self.ffmpeg;
        let gradient = self.gradient.clone();
        let mut close = false;
        let mut start: Option<(MediaInfo, Quality)> = None;

        let modal = egui::Modal::new(egui::Id::unique("picker"))
            .frame(Frame::new().fill(p.surface).corner_radius(18).inner_margin(Margin::same(22)).stroke(Stroke::new(1.0, p.border)))
            .show(ctx, |ui| {
                ui.set_width(560.0);
                let m = &picker.media;
                ui.horizontal(|ui| {
                    Self::thumbnail(ui, &m.thumbnail, vec2(208.0, 117.0), 12, p.surface_hover);
                    ui.vertical(|ui| {
                        ui.set_width(320.0);
                        ui.add(egui::Label::new(RichText::new(&m.title).size(17.0).strong()).wrap());
                        if !m.uploader.is_empty() {
                            ui.label(RichText::new(&m.uploader).color(p.muted));
                        }
                        ui.horizontal(|ui| {
                            if !m.site.is_empty() {
                                Self::chip(ui, &m.site, ACCENT_A.gamma_multiply(0.18), ACCENT_A);
                            }
                            if let Some(d) = m.duration {
                                Self::chip(ui, &fmt_duration(d), p.surface_hover, p.muted);
                            }
                        });
                    });
                });
                ui.add_space(10.0);

                if picker.loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Carregando formatos disponíveis…").color(p.muted));
                    });
                } else if let Some(err) = &picker.error {
                    ui.label(RichText::new(format!("⚠ {err}")).color(DANGER));
                } else {
                    let heights: Vec<u32> = if m.heights.is_empty() { vec![1080, 720, 480, 360] } else { m.heights.clone() };
                    ui.label(RichText::new("🎬  Vídeo").strong());
                    ui.horizontal_wrapped(|ui| {
                        for h in heights {
                            let size = m.sizes.iter().find(|(sh, _)| *sh == h).map(|(_, s)| *s + m.audio_size.unwrap_or(0));
                            let q = Quality::Video(h);
                            let sub = size.map(fmt_bytes).unwrap_or_else(|| "MP4".into());
                            let limited = !ffmpeg && h > 720;
                            if option_tile(ui, &p, picker.quality == q, &engine::height_label(h), &sub, limited).clicked() {
                                picker.quality = q;
                            }
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new("🎵  Somente áudio").strong());
                    ui.horizontal_wrapped(|ui| {
                        let audio = m.audio_size.map(fmt_bytes);
                        let mp3_sub = if ffmpeg { "Alta qualidade".to_string() } else { "requer ffmpeg".into() };
                        if option_tile(ui, &p, picker.quality == Quality::Mp3, "MP3", &mp3_sub, !ffmpeg).clicked() {
                            picker.quality = Quality::Mp3;
                        }
                        let m4a_sub = audio.unwrap_or_else(|| "Original".into());
                        if option_tile(ui, &p, picker.quality == Quality::M4a, "M4A", &m4a_sub, false).clicked() {
                            picker.quality = Quality::M4a;
                        }
                    });
                    if !ffmpeg {
                        ui.label(
                            RichText::new("Sem ffmpeg: vídeos acima de 720p e MP3 ficam limitados. Instale-o em Configurações.")
                                .size(12.5)
                                .color(WARNING),
                        );
                    }
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let enabled = !picker.loading && picker.error.is_none();
                        let (rect, resp) = ui.allocate_exact_size(vec2(170.0, 42.0), Sense::click());
                        if enabled {
                            theme::gradient_rect(ui.painter(), &gradient, rect, CornerRadius::same(10));
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, 10, Color32::from_white_alpha(28));
                            }
                        } else {
                            ui.painter().rect_filled(rect, 10, p.border);
                        }
                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "⬇  Baixar agora", FontId::proportional(15.5), Color32::WHITE);
                        if enabled && resp.clicked() {
                            start = Some((picker.media.clone(), picker.quality));
                        }
                        if ui.add(egui::Button::new("Cancelar").min_size(vec2(110.0, 42.0))).clicked() {
                            close = true;
                        }
                        if !picker.loading && picker.error.is_none() {
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.label(RichText::new(picker.quality.label()).color(p.muted));
                            });
                        }
                    });
                });
            });
        if modal.should_close() {
            close = true;
        }
        if let Some((m, q)) = start {
            self.picker = None;
            self.enqueue(m, q);
        } else if close {
            self.picker = None;
        }
    }

    fn downloads(&mut self, ui: &mut Ui) {
        let p = self.pal();
        let running = self.tasks.iter().filter(|t| t.state == TaskState::Running).count();
        let queued = self.tasks.iter().filter(|t| t.state == TaskState::Queued).count();
        let done = self.tasks.iter().filter(|t| matches!(t.state, TaskState::Done(_))).count();

        ui.horizontal(|ui| {
            ui.heading("Downloads");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let finished = self.tasks.iter().any(|t| !matches!(t.state, TaskState::Running | TaskState::Queued));
                if ui.add_enabled(finished, egui::Button::new("🧹  Limpar finalizados")).clicked() {
                    self.tasks.retain(|t| matches!(t.state, TaskState::Running | TaskState::Queued));
                }
                if ui.button("📁  Abrir pasta").clicked() {
                    let _ = std::fs::create_dir_all(&self.settings.download_dir);
                    let _ = open::that(&self.settings.download_dir);
                }
            });
        });
        let stats = [("Em andamento", running, ACCENT_A), ("Na fila", queued, WARNING), ("Concluídos", done, SUCCESS)];
        self.card_row(ui, 3, 0.0, |i, ui| {
            let (label, n, color) = stats[i];
            ui.label(RichText::new(n.to_string()).size(26.0).strong().color(color));
            ui.label(RichText::new(label).color(p.muted));
        });
        ui.add_space(6.0);

        if self.tasks.is_empty() {
            empty_state(ui, &p, "⬇", "Nenhum download por aqui", "Pesquise ou cole um link na tela Início para começar.");
            return;
        }

        enum Action {
            Cancel(u64),
            Retry(u64),
            Remove(u64),
        }
        let mut action = None;
        for task in &self.tasks {
            self.card().inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    Self::thumbnail(ui, &task.media.thumbnail, vec2(144.0, 81.0), 10, p.surface_hover);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width() - 150.0);
                        ui.add(egui::Label::new(RichText::new(&task.media.title).size(15.0).strong()).truncate());
                        ui.horizontal(|ui| {
                            Self::chip(ui, &task.quality.label(), ACCENT_A.gamma_multiply(0.16), ACCENT_A);
                            if !task.media.uploader.is_empty() {
                                ui.label(RichText::new(&task.media.uploader).size(12.5).color(p.muted));
                            }
                        });
                        match &task.state {
                            TaskState::Queued => {
                                self.progress_bar(ui, 0.0, Some(p.border));
                                ui.label(RichText::new("Na fila…").size(12.5).color(p.muted));
                            }
                            TaskState::Running => {
                                self.progress_bar(ui, task.progress.fraction, None);
                                let pr = &task.progress;
                                let mut parts = vec![format!("{} {:.0}%", pr.stage, pr.fraction * 100.0)];
                                for (s, prefix) in [(&pr.speed, ""), (&pr.eta, "faltam "), (&pr.total, "")] {
                                    if !s.is_empty() {
                                        parts.push(format!("{prefix}{s}"));
                                    }
                                }
                                ui.label(RichText::new(parts.join("  ·  ")).size(12.5).color(p.muted));
                            }
                            TaskState::Done(path) => {
                                self.progress_bar(ui, 1.0, Some(SUCCESS));
                                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                ui.label(RichText::new(format!("✔ Concluído  ·  {name}")).size(12.5).color(SUCCESS));
                            }
                            TaskState::Failed(e) => {
                                self.progress_bar(ui, task.progress.fraction, Some(DANGER));
                                ui.add(egui::Label::new(RichText::new(format!("✖ {e}")).size(12.5).color(DANGER)).truncate());
                            }
                            TaskState::Canceled => {
                                self.progress_bar(ui, task.progress.fraction, Some(p.muted));
                                ui.label(RichText::new("Cancelado").size(12.5).color(p.muted));
                            }
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        match &task.state {
                            TaskState::Running | TaskState::Queued => {
                                if icon_button(ui, &p, "✖", "Cancelar").clicked() {
                                    action = Some(Action::Cancel(task.id));
                                }
                            }
                            TaskState::Done(path) => {
                                if icon_button(ui, &p, "🗑", "Remover da lista").clicked() {
                                    action = Some(Action::Remove(task.id));
                                }
                                if icon_button(ui, &p, "📁", "Mostrar na pasta").clicked() {
                                    let _ = open::that(path.parent().unwrap_or(path));
                                }
                                if icon_button(ui, &p, "▶", "Abrir").clicked() {
                                    let _ = open::that(path);
                                }
                            }
                            TaskState::Failed(_) | TaskState::Canceled => {
                                if icon_button(ui, &p, "🗑", "Remover da lista").clicked() {
                                    action = Some(Action::Remove(task.id));
                                }
                                if icon_button(ui, &p, "⟳", "Tentar novamente").clicked() {
                                    action = Some(Action::Retry(task.id));
                                }
                            }
                        }
                    });
                });
            });
        }
        match action {
            Some(Action::Cancel(id)) => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                    engine::kill(&t.child);
                    t.state = TaskState::Canceled;
                }
            }
            Some(Action::Retry(id)) => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                    t.state = TaskState::Queued;
                    t.progress = Progress::default();
                }
            }
            Some(Action::Remove(id)) => self.tasks.retain(|t| t.id != id),
            None => {}
        }
    }

    fn library(&mut self, ui: &mut Ui) {
        let p = self.pal();
        ui.horizontal(|ui| {
            ui.heading("Biblioteca");
            ui.label(RichText::new(format!("{} itens", self.history.len())).color(p.muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                Frame::new().fill(p.surface).stroke(Stroke::new(1.0, p.border)).corner_radius(10).inner_margin(Margin::symmetric(10, 2)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("🔍").color(p.muted));
                        ui.add(egui::TextEdit::singleline(&mut self.library_filter).hint_text("Filtrar…").frame(Frame::NONE).desired_width(200.0));
                    });
                });
            });
        });
        ui.add_space(4.0);
        if self.history.is_empty() {
            empty_state(ui, &p, "📚", "Sua biblioteca está vazia", "Os vídeos e músicas que você baixar aparecem aqui.");
            return;
        }
        let filter = self.library_filter.to_lowercase();
        let mut remove = None;
        let visible: Vec<usize> = (0..self.history.len())
            .filter(|&i| {
                let item = &self.history[i];
                filter.is_empty() || item.title.to_lowercase().contains(&filter) || item.uploader.to_lowercase().contains(&filter)
            })
            .collect();
        let card = self.card().inner_margin(Margin::same(10));
        let history = &self.history;
        grid(ui, visible.len(), 200.0, 16.0, |ui, n, cell_w| {
            let i = visible[n];
            let item = &history[i];
            let card_w = cell_w - 22.0; // margens internas + borda
            {
                let exists = item.path.exists();
                card.show(ui, |ui| ui.vertical(|ui| {
                    ui.set_width(card_w);
                    ui.spacing_mut().item_spacing.y = 5.0;
                    let img = Self::thumbnail(ui, &item.thumbnail, vec2(card_w, card_w * 9.0 / 16.0), 10, p.surface_hover);
                    if item.quality.is_audio() {
                        ui.painter().circle_filled(img.rect.left_top() + vec2(22.0, 22.0), 15.0, Color32::from_black_alpha(170));
                        ui.painter().text(img.rect.left_top() + vec2(22.0, 22.0), Align2::CENTER_CENTER, "🎵", FontId::proportional(14.0), Color32::WHITE);
                    }
                    if img.clicked() && exists {
                        let _ = open::that(&item.path);
                    }
                    ui.add(egui::Label::new(RichText::new(&item.title).strong()).truncate());
                    ui.horizontal(|ui| {
                        Self::chip(ui, &item.quality.label(), p.surface_hover, p.muted);
                        if !item.size.is_empty() {
                            ui.label(RichText::new(&item.size).size(12.0).color(p.muted));
                        }
                    });
                    ui.horizontal(|ui| {
                        if !exists {
                            ui.label(RichText::new("Arquivo não encontrado").size(12.0).color(WARNING));
                        } else {
                            if icon_button(ui, &p, "▶", "Abrir").clicked() {
                                let _ = open::that(&item.path);
                            }
                            if icon_button(ui, &p, "📁", "Mostrar na pasta").clicked() {
                                let _ = open::that(item.path.parent().unwrap_or(&item.path));
                            }
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if icon_button(ui, &p, "🗑", "Remover da biblioteca (não apaga o arquivo)").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                }));
            }
        });
        if let Some(i) = remove {
            self.history.remove(i);
            store::save_history(&self.history);
        }
    }

    fn settings_page(&mut self, ui: &mut Ui) {
        let p = self.pal();
        ui.heading("Configurações");
        ui.add_space(4.0);
        let mut changed = false;

        self.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("📁  Downloads").size(16.0).strong());
            ui.add_space(4.0);
            ui.label(RichText::new("Pasta onde os arquivos são salvos").color(p.muted));
            ui.horizontal(|ui| {
                let mut text = self.settings.download_dir.to_string_lossy().to_string();
                if ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width() - 120.0)).changed() {
                    self.settings.download_dir = PathBuf::from(text);
                    changed = true;
                }
                if ui.button("Abrir").clicked() {
                    let _ = std::fs::create_dir_all(&self.settings.download_dir);
                    let _ = open::that(&self.settings.download_dir);
                }
            });
            ui.add_space(6.0);
            egui::Grid::new("settings-grid").num_columns(2).spacing(vec2(24.0, 14.0)).show(ui, |ui| {
                ui.label("Qualidade padrão");
                egui::ComboBox::from_id_salt("quality")
                    .width(180.0)
                    .selected_text(self.settings.default_quality.label())
                    .show_ui(ui, |ui| {
                        for q in [
                            Quality::Video(2160),
                            Quality::Video(1440),
                            Quality::Video(1080),
                            Quality::Video(720),
                            Quality::Video(480),
                            Quality::Video(360),
                            Quality::Mp3,
                            Quality::M4a,
                        ] {
                            changed |= ui.selectable_value(&mut self.settings.default_quality, q, q.label()).changed();
                        }
                    });
                ui.end_row();
                ui.label("Downloads simultâneos");
                changed |= ui.add(egui::Slider::new(&mut self.settings.max_parallel, 1..=6)).changed();
                ui.end_row();
                ui.label("Resultados por busca");
                changed |= ui.add(egui::Slider::new(&mut self.settings.search_results, 6..=40)).changed();
                ui.end_row();
                ui.label("Tema");
                ui.horizontal(|ui| {
                    changed |= ui.selectable_value(&mut self.settings.dark, true, "🌙 Escuro").changed();
                    changed |= ui.selectable_value(&mut self.settings.dark, false, "☀ Claro").changed();
                });
                ui.end_row();
            });
        });

        self.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("⚙  Motor de downloads").size(16.0).strong());
            ui.add_space(4.0);
            let busy = self.installing.is_some();
            let mut do_install: Option<bool> = None;
            let mut do_update = false;
            egui::Grid::new("tools-grid").num_columns(3).spacing(vec2(24.0, 14.0)).show(ui, |ui| {
                ui.label(RichText::new("yt-dlp").strong());
                match &self.ytdlp {
                    Some(v) => ui.label(RichText::new(format!("✔ versão {v}")).color(SUCCESS)),
                    None => ui.label(RichText::new("✖ não instalado").color(DANGER)),
                };
                ui.add_enabled_ui(!busy, |ui| {
                    if self.ytdlp.is_some() {
                        do_update = ui.button("⟳  Atualizar").clicked();
                    } else {
                        do_install = ui.button("⬇  Instalar").clicked().then_some(false);
                    }
                });
                ui.end_row();
                ui.label(RichText::new("ffmpeg").strong());
                if self.ffmpeg {
                    ui.label(RichText::new("✔ encontrado").color(SUCCESS));
                } else {
                    ui.label(RichText::new("✖ não instalado (necessário p/ 1080p+ e MP3)").color(WARNING));
                }
                ui.add_enabled_ui(!busy, |ui| {
                    if !self.ffmpeg && ui.button("⬇  Instalar").clicked() {
                        do_install = Some(true);
                    }
                });
                ui.end_row();
            });
            if let Some((text, f)) = &self.installing {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if *f >= 0.0 { format!("{text} {:.0}%", f * 100.0) } else { text.clone() });
                });
                if *f >= 0.0 {
                    let f = *f;
                    self.progress_bar(ui, f, None);
                }
            }
            ui.label(
                RichText::new(format!("As ferramentas baixadas ficam em {}", engine::tools_dir().display()))
                    .size(12.0)
                    .color(p.muted),
            );
            if let Some(ff) = do_install {
                self.install_tool(ff);
            }
            if do_update {
                self.update_ytdlp();
            }
        });

        self.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("ℹ  Sobre").size(16.0).strong());
            ui.label(RichText::new(format!("Fluxo {} — interface desktop em Rust para o yt-dlp.", env!("CARGO_PKG_VERSION"))).color(p.muted));
            ui.label(
                RichText::new("Baixe apenas conteúdo que você tem direito de baixar e respeite os termos de uso de cada site.")
                    .size(12.5)
                    .color(p.muted),
            );
        });

        if changed {
            store::save_settings(&self.settings);
        }
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let Some(toast) = &self.toast else { return };
        if now > toast.until {
            self.toast = None;
            return;
        }
        let p = self.pal();
        let alpha = ((toast.until - now) / 0.4).clamp(0.0, 1.0) as f32;
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-24.0, -24.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, toast.color.gamma_multiply(0.7)))
                    .corner_radius(12)
                    .inner_margin(Margin::symmetric(16, 12))
                    .shadow(egui::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(90) })
                    .show(ui, |ui| {
                        ui.set_max_width(380.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 5.0, toast.color);
                            ui.add(egui::Label::new(&toast.text).wrap());
                        });
                    });
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

impl eframe::App for FluxoApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied_dark != Some(self.settings.dark) {
            theme::apply(&ctx, self.settings.dark);
            self.applied_dark = Some(self.settings.dark);
        }
        self.handle_messages();
        if self.pending_submit && self.tools_checked {
            self.pending_submit = false;
            self.submit_query();
        }
        self.schedule();
        let p = self.pal();

        egui::Panel::left("nav")
            .exact_size(232.0)
            .resizable(false)
            .frame(Frame::new().fill(p.sidebar).inner_margin(Margin::symmetric(14, 0)).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| self.sidebar(ui));

        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(28, 22)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.page {
                    Page::Home => self.home(ui),
                    Page::Downloads => self.downloads(ui),
                    Page::Library => self.library(ui),
                    Page::Settings => self.settings_page(ui),
                });
            });

        self.picker_modal(&ctx);
        self.toast_ui(&ctx);
    }
}

// ------------------------------------------------------------------ helpers

/// Grade responsiva: tantas colunas de largura >= `min_w` quanto couberem.
fn grid(ui: &mut Ui, count: usize, min_w: f32, gap: f32, mut cell: impl FnMut(&mut Ui, usize, f32)) {
    let avail = ui.available_width();
    let cols = (((avail + gap) / (min_w + gap)).floor() as usize).max(1);
    let w = (avail - gap * (cols - 1) as f32) / cols as f32;
    let mut start = 0;
    while start < count {
        let end = (start + cols).min(count);
        let top_left = ui.cursor().min;
        let mut height: f32 = 0.0;
        for (col, i) in (start..end).enumerate() {
            let rect = egui::Rect::from_min_size(top_left + vec2(col as f32 * (w + gap), 0.0), vec2(w, f32::INFINITY));
            let mut child = ui.new_child(
                egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)).id_salt(("grid-cell", i)),
            );
            cell(&mut child, i, w);
            height = height.max(child.min_rect().height());
        }
        ui.allocate_exact_size(vec2(avail, height), Sense::hover());
        ui.add_space(gap);
        start = end;
    }
}

fn option_tile(ui: &mut Ui, p: &Palette, selected: bool, title: &str, sub: &str, warn: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(96.0, 54.0), Sense::click());
    let painter = ui.painter();
    let (fill, stroke) = if selected {
        (ACCENT_A.gamma_multiply(0.16), Stroke::new(1.5, ACCENT_A))
    } else if resp.hovered() {
        (p.surface_hover, Stroke::new(1.0, ACCENT_A.gamma_multiply(0.5)))
    } else {
        (p.surface_hover, Stroke::new(1.0, p.border))
    };
    painter.rect(rect, 10, fill, stroke, egui::StrokeKind::Inside);
    painter.text(
        rect.center_top() + vec2(0.0, 17.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(15.5),
        if selected { ACCENT_A } else { p.text },
    );
    painter.text(
        rect.center_bottom() - vec2(0.0, 14.0),
        Align2::CENTER_CENTER,
        sub,
        FontId::proportional(11.0),
        if warn { WARNING } else { p.muted },
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn icon_button(ui: &mut Ui, p: &Palette, icon: &str, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 9, p.surface_hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(16.0),
        if resp.hovered() { ACCENT_B } else { p.muted },
    );
    resp.on_hover_text(tip)
}

fn empty_state(ui: &mut Ui, p: &Palette, icon: &str, title: &str, text: &str) {
    ui.add_space(50.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(84.0, 84.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 42.0, ACCENT_A.gamma_multiply(0.14));
        ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(34.0), ACCENT_B);
        ui.add_space(6.0);
        ui.label(RichText::new(title).size(18.0).strong());
        ui.label(RichText::new(text).color(p.muted));
    });
}

fn fmt_duration(secs: f64) -> String {
    let s = secs.round() as u64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn fmt_views(v: u64) -> String {
    let n = v as f64;
    let s = if n >= 1e9 {
        format!("{:.1} bi", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.1} mi", n / 1e6)
    } else if n >= 1e3 {
        format!("{:.0} mil", n / 1e3)
    } else {
        v.to_string()
    };
    format!("{} visualizações", s.replace('.', ","))
}

fn fmt_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
    .replace('.', ",")
}
