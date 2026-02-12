mod chat;
mod clipboard;
mod config;
mod glossary;
mod memory;
mod offsets;
mod player;
mod translation;
mod wtf_parser;

use std::collections::HashMap;
use std::num::NonZeroU32;

use chat::{ChatMessage, ChatReader, ChatTab, TextSegment};
use translation::{TranslationEntry, TranslationRequest, TranslationResponse, TranslationService};
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::prelude::GlSurface;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::DisplayBuilder;
use imgui_glow_renderer::AutoRenderer;
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use log::{error, info, warn};
use raw_window_handle::HasWindowHandle;
use sysinfo::System;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::EventLoop;
use winit::window::{Window, WindowAttributes};

const MAX_MESSAGES: usize = 500;
const THEMES: &[&str] = &["Dark", "Light", "Classic"];

#[derive(PartialEq, Clone, Copy)]
enum AppBarDropdown {
    Process,
    Settings,
    DebugTools,
}

// ─── App State ───────────────────────────────────────────────────────

struct AppState {
    // AppBar dropdown state
    open_dropdown: Option<AppBarDropdown>,

    // Persisted config
    config: config::AppConfig,

    // Runtime state
    status_text: String,
    attached_pid: Option<u32>,
    reader: Box<dyn memory::ProcessMemoryReader>,
    chat_reader: ChatReader,
    chat_messages: Vec<ChatMessage>,
    chat_tabs: Vec<ChatTab>,
    active_tab: usize,
    had_new_messages: bool,
    search_text: String,
    clipboard: Option<clipboard::ClipboardHelper>,

    // Player info (read from memory each frame)
    player_info: Option<player::PlayerInfo>,

    // Translation
    translation_service: Option<TranslationService>,
    translation_rx: Option<std::sync::mpsc::Receiver<TranslationResponse>>,
    translations: HashMap<u64, TranslationEntry>,
    auto_translate: bool,
    target_languages: Vec<(String, String)>,
    translation_error: String,
    api_key_input: String,

    // Settings UI
    available_fonts: Vec<config::FontEntry>,
    character_configs: Vec<wtf_parser::CharacterConfig>,
    selected_char_index: usize,
    loaded_wtf_tabs: Option<Vec<ChatTab>>,
    wtf_status: String,
    font_changed: bool,
    theme_changed: bool,

    // Translator window
    translator_window_open: bool,
    translator_input: String,
    translator_output: String,
    translator_pending: bool,
    translator_error: String,

    // Glossary
    glossary: glossary::Glossary,
    glossary_editor_open: bool,
    glossary_edit_keys: String,
    glossary_edit_description_en: String,
    glossary_edit_description_ru: String,
    glossary_editing_index: Option<usize>,
    glossary_editor_status: String,
}

// ─── App (owns GL + imgui state) ─────────────────────────────────────

struct App {
    window: Option<Window>,
    gl_config: Option<glutin::config::Config>,
    gl_context: Option<PossiblyCurrentContext>,
    gl_surface: Option<Surface<WindowSurface>>,
    glow_context: Option<glow::Context>,
    imgui: Option<imgui::Context>,
    platform: Option<WinitPlatform>,
    renderer: Option<AutoRenderer>,
    state: AppState,
}

impl App {
    fn new() -> Self {
        let cfg = config::AppConfig::load();
        let available_fonts = config::discover_system_fonts();

        // Auto-restore saved character profile and chat filters.
        let mut character_configs = Vec::new();
        let mut selected_char_index = 0;
        let mut chat_tabs = chat::default_tabs();
        let mut wtf_status = String::new();

        if !cfg.wow_folder_path.is_empty() {
            let path = std::path::Path::new(&cfg.wow_folder_path);
            if let Ok(configs) = wtf_parser::find_character_configs(path) {
                if !configs.is_empty() && !cfg.selected_character.is_empty() {
                    if let Some(idx) = configs
                        .iter()
                        .position(|c| c.display_label() == cfg.selected_character)
                    {
                        selected_char_index = idx;
                        match wtf_parser::parse_chat_cache(&configs[idx].chat_cache_path) {
                            Ok(windows) => {
                                let tabs = wtf_parser::to_chat_tabs(&windows);
                                info!(
                                    "Auto-loaded {} chat tabs for {}",
                                    tabs.len(),
                                    cfg.selected_character,
                                );
                                wtf_status = format!(
                                    "Auto-loaded {} tabs from {}",
                                    tabs.len(),
                                    configs[idx].character,
                                );
                                chat_tabs = tabs;
                            }
                            Err(e) => {
                                warn!("Auto-load chat config failed: {}", e);
                                wtf_status = format!("Auto-load error: {}", e);
                            }
                        }
                    }
                }
                character_configs = configs;
            }
        }

        // Start translation service if API key is configured
        let auto_translate = cfg.auto_translate;
        let api_key_input = cfg.deepl_api_key.clone();
        let (translation_service, translation_rx) = if !cfg.deepl_api_key.is_empty() {
            let (service, rx) =
                TranslationService::start(cfg.deepl_api_key.clone(), cfg.target_language.clone());
            service.fetch_languages();
            (Some(service), Some(rx))
        } else {
            (None, None)
        };

        Self {
            window: None,
            gl_config: None,
            gl_context: None,
            gl_surface: None,
            glow_context: None,
            imgui: None,
            platform: None,
            renderer: None,
            state: AppState {
                open_dropdown: None,
                config: cfg,
                status_text: String::from("Not attached"),
                attached_pid: None,
                reader: memory::create_reader(),
                chat_reader: ChatReader::new(),
                player_info: None,
                chat_messages: Vec::new(),
                chat_tabs,
                active_tab: 0,
                had_new_messages: false,
                search_text: String::new(),
                clipboard: clipboard::ClipboardHelper::new(),
                translation_service,
                translation_rx,
                translations: HashMap::new(),
                auto_translate,
                target_languages: Vec::new(),
                translation_error: String::new(),
                api_key_input,
                available_fonts,
                character_configs,
                selected_char_index,
                loaded_wtf_tabs: None,
                wtf_status,
                font_changed: false,
                theme_changed: false,
                translator_window_open: false,
                translator_input: String::new(),
                translator_output: String::new(),
                translator_pending: false,
                translator_error: String::new(),
                glossary: glossary::Glossary::load(),
                glossary_editor_open: false,
                glossary_edit_keys: String::new(),
                glossary_edit_description_en: String::new(),
                glossary_edit_description_ru: String::new(),
                glossary_editing_index: None,
                glossary_editor_status: String::new(),
            },
        }
    }

    /// Rebuild the imgui font atlas with the current config settings.
    fn rebuild_fonts(&mut self) {
        let Some(imgui) = self.imgui.as_mut() else {
            return;
        };
        let Some(gl_config) = self.gl_config.as_ref() else {
            return;
        };

        imgui.fonts().clear();
        load_font(
            imgui,
            &self.state.config.font_name,
            &self.state.available_fonts,
            self.state.config.font_size,
        );

        let gl_display = gl_config.display();
        let new_glow = unsafe {
            glow::Context::from_loader_function_cstr(|name| gl_display.get_proc_address(name))
        };

        // Drop old renderer before creating new one.
        self.renderer = None;
        self.renderer = Some(
            AutoRenderer::new(new_glow, imgui).expect("Failed to recreate renderer"),
        );
        info!("Font atlas rebuilt");
    }
}

// ─── Font / theme helpers ────────────────────────────────────────────

fn load_font(
    imgui: &mut imgui::Context,
    font_name: &str,
    fonts: &[config::FontEntry],
    size: f32,
) {
    // Find full path from discovered fonts.
    let font_path = fonts
        .iter()
        .find(|f| f.name == font_name)
        .map(|f| f.path.as_str());

    // Fallback list if saved font not found.
    let fallback = [
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ];

    let path = font_path.or_else(|| {
        fallback
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .copied()
    });

    if let Some(path) = path {
        if let Ok(font_data) = std::fs::read(path) {
            let font_data: &'static [u8] = Vec::leak(font_data);
            imgui.fonts().add_font(&[imgui::FontSource::TtfData {
                data: font_data,
                size_pixels: size,
                config: Some(imgui::FontConfig {
                    glyph_ranges: imgui::FontGlyphRanges::cyrillic(),
                    ..Default::default()
                }),
            }]);
            info!("Loaded font: {} (size {:.0})", path, size);
            return;
        }
    }

    warn!(
        "Failed to load font '{}', using imgui default",
        font_name
    );
}

fn apply_theme(imgui: &mut imgui::Context, theme: &str) {
    let style = imgui.style_mut();
    match theme {
        "Light" => style.use_light_colors(),
        "Classic" => style.use_classic_colors(),
        _ => style.use_dark_colors(),
    };
}

// ─── ApplicationHandler ──────────────────────────────────────────────

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_attrs = WindowAttributes::default()
            .with_title("WotLK Chat Translator")
            .with_inner_size(winit::dpi::LogicalSize::new(1100.0f32, 750.0));

        let config_template = ConfigTemplateBuilder::new();
        let display_builder = DisplayBuilder::new().with_window_attributes(Some(window_attrs));

        let (window, gl_config) = display_builder
            .build(event_loop, config_template, |mut configs| {
                configs.next().unwrap()
            })
            .expect("Failed to build display");

        let window = window.expect("Failed to create window");
        let gl_display = gl_config.display();
        let context_attrs = ContextAttributesBuilder::new().build(Some(
            window
                .window_handle()
                .expect("Failed to get window handle")
                .into(),
        ));

        let gl_context = unsafe {
            gl_display
                .create_context(&gl_config, &context_attrs)
                .expect("Failed to create GL context")
        };

        let size = window.inner_size();
        let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            window
                .window_handle()
                .expect("Failed to get window handle")
                .into(),
            NonZeroU32::new(size.width.max(1)).unwrap(),
            NonZeroU32::new(size.height.max(1)).unwrap(),
        );

        let gl_surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surface_attrs)
                .expect("Failed to create GL surface")
        };

        let gl_context = gl_context
            .make_current(&gl_surface)
            .expect("Failed to make GL context current");

        let _ = gl_surface.set_swap_interval(
            &gl_context,
            SwapInterval::Wait(NonZeroU32::new(1).unwrap()),
        );

        let glow_context = unsafe {
            glow::Context::from_loader_function_cstr(|name| gl_display.get_proc_address(name))
        };

        let mut imgui = imgui::Context::create();

        // Enable Ctrl+V paste in imgui input fields.
        if let Some(backend) = clipboard::ImguiClipboardBackend::new() {
            imgui.set_clipboard_backend(backend);
        }

        // Persist imgui layout (window positions, sizes, collapsed state).
        imgui.set_ini_filename(Some(config::config_dir().join("imgui_layout.ini")));

        // Load font from config.
        load_font(
            &mut imgui,
            &self.state.config.font_name,
            &self.state.available_fonts,
            self.state.config.font_size,
        );

        // Apply saved theme.
        apply_theme(&mut imgui, &self.state.config.theme);

        let mut platform = WinitPlatform::new(&mut imgui);
        platform.attach_window(imgui.io_mut(), &window, HiDpiMode::Default);

        let renderer =
            AutoRenderer::new(glow_context, &mut imgui).expect("Failed to create renderer");

        let glow_context = unsafe {
            glow::Context::from_loader_function_cstr(|name| gl_display.get_proc_address(name))
        };

        self.window = Some(window);
        self.gl_config = Some(gl_config);
        self.gl_context = Some(gl_context);
        self.gl_surface = Some(gl_surface);
        self.glow_context = Some(glow_context);
        self.imgui = Some(imgui);
        self.platform = Some(platform);
        self.renderer = Some(renderer);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        // Forward event to imgui platform.
        if let (Some(window), Some(imgui), Some(platform)) = (
            self.window.as_ref(),
            self.imgui.as_mut(),
            self.platform.as_mut(),
        ) {
            platform.handle_event::<()>(
                imgui.io_mut(),
                window,
                &winit::event::Event::WindowEvent {
                    window_id: _window_id,
                    event: event.clone(),
                },
            );
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                if let (Some(gl_surface), Some(gl_context)) =
                    (self.gl_surface.as_ref(), self.gl_context.as_ref())
                {
                    gl_surface.resize(
                        gl_context,
                        NonZeroU32::new(new_size.width.max(1)).unwrap(),
                        NonZeroU32::new(new_size.height.max(1)).unwrap(),
                    );
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                // ── Hot-reload font atlas if needed ───────────────
                if self.state.font_changed {
                    self.rebuild_fonts();
                    self.state.font_changed = false;
                }
                if self.state.theme_changed {
                    if let Some(imgui) = self.imgui.as_mut() {
                        apply_theme(imgui, &self.state.config.theme);
                    }
                    self.state.theme_changed = false;
                }

                // ── Get rendering references ─────────────────────
                let (
                    Some(window),
                    Some(imgui),
                    Some(platform),
                    Some(renderer),
                    Some(gl_context),
                    Some(gl_surface),
                    Some(glow_ctx),
                ) = (
                    self.window.as_ref(),
                    self.imgui.as_mut(),
                    self.platform.as_mut(),
                    self.renderer.as_mut(),
                    self.gl_context.as_ref(),
                    self.gl_surface.as_ref(),
                    self.glow_context.as_ref(),
                )
                else {
                    return;
                };

                // ── Poll for new chat messages ───────────────────
                let state = &mut self.state;
                state.had_new_messages = false;

                if state.attached_pid.is_some() {
                    match state.chat_reader.poll(&*state.reader) {
                        Ok(new_msgs) => {
                            if !new_msgs.is_empty() {
                                state.had_new_messages = true;
                                append_chat_history(&new_msgs);

                                // Auto-translate new messages before adding to history
                                if state.auto_translate {
                                    if let Some(ref service) = state.translation_service {
                                        for msg in &new_msgs {
                                            let (text, link_names) =
                                                translation::prepare_for_translation(&msg.segments);
                                            if !text.trim().is_empty() {
                                                state
                                                    .translations
                                                    .insert(msg.id, TranslationEntry::Pending);
                                                service.translate(TranslationRequest {
                                                    message_id: msg.id,
                                                    text,
                                                    link_names,
                                                    source_lang: None,
                                                    target_lang: None,
                                                });
                                            }
                                        }
                                    }
                                }

                                state.chat_messages.extend(new_msgs);
                                if state.chat_messages.len() > MAX_MESSAGES {
                                    let drain = state.chat_messages.len() - MAX_MESSAGES;
                                    state.chat_messages.drain(..drain);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Poll failed, auto-detaching: {}", e);
                            state.status_text = format!("Read error (detached): {}", e);
                            state.attached_pid = None;
                            let _ = state.reader.detach();
                            state.chat_reader.reset();
                        }
                    }
                    // Read player info (name, realm, level, money) from memory.
                    state.player_info = player::read_player_info(&*state.reader);
                } else {
                    state.player_info = None;
                }

                // Poll translation responses (non-blocking)
                if let Some(ref rx) = state.translation_rx {
                    while let Ok(resp) = rx.try_recv() {
                        match resp {
                            TranslationResponse::Success {
                                message_id,
                                translated,
                            } => {
                                if message_id == u64::MAX {
                                    state.translator_output = translated;
                                    state.translator_pending = false;
                                    state.translator_error.clear();
                                } else {
                                    state
                                        .translations
                                        .insert(message_id, TranslationEntry::Done(translated));
                                }
                            }
                            TranslationResponse::Error { message_id, error } => {
                                if message_id == u64::MAX {
                                    state.translator_error = error;
                                    state.translator_pending = false;
                                    state.translator_output.clear();
                                } else {
                                    error!("Translation error for msg {}: {}", message_id, error);
                                    state
                                        .translations
                                        .insert(message_id, TranslationEntry::Error(error));
                                }
                            }
                            TranslationResponse::Languages(langs) => {
                                state.target_languages = langs;
                                state.translation_error.clear();
                            }
                            TranslationResponse::LanguagesError(e) => {
                                state.translation_error =
                                    format!("Failed to fetch languages: {}", e);
                            }
                        }
                    }
                }

                // ── Render UI ────────────────────────────────────
                platform
                    .prepare_frame(imgui.io_mut(), window)
                    .expect("Failed to prepare frame");

                let ui = imgui.frame();
                let state = &mut self.state;
                let is_attached = state.attached_pid.is_some();

                // ── AppBar ────────────────────────────────────────
                let mut appbar_height = 35.0_f32;
                let mut appbar_hovered = false;
                let display_size = ui.io().display_size;

                ui.window("##appbar")
                    .position([0.0, 0.0], imgui::Condition::Always)
                    .size([display_size[0], 0.0], imgui::Condition::Always)
                    .flags(
                        imgui::WindowFlags::NO_TITLE_BAR
                            | imgui::WindowFlags::NO_RESIZE
                            | imgui::WindowFlags::NO_MOVE
                            | imgui::WindowFlags::NO_SCROLLBAR
                            | imgui::WindowFlags::NO_SCROLL_WITH_MOUSE
                            | imgui::WindowFlags::NO_COLLAPSE
                            | imgui::WindowFlags::NO_SAVED_SETTINGS
                            | imgui::WindowFlags::ALWAYS_AUTO_RESIZE,
                    )
                    .build(|| {
                        let toggle = |dropdown: AppBarDropdown,
                                      current: &mut Option<AppBarDropdown>| {
                            if *current == Some(dropdown) {
                                *current = None;
                            } else {
                                *current = Some(dropdown);
                            }
                        };

                        if ui.button("Process") {
                            toggle(AppBarDropdown::Process, &mut state.open_dropdown);
                        }
                        ui.same_line();
                        if ui.button("Settings") {
                            toggle(AppBarDropdown::Settings, &mut state.open_dropdown);
                        }
                        ui.same_line();
                        if ui.button("Debug Tools") {
                            toggle(AppBarDropdown::DebugTools, &mut state.open_dropdown);
                        }
                        ui.same_line();
                        if ui.button("Glossary") {
                            state.glossary_editor_open = !state.glossary_editor_open;
                        }
                        ui.same_line();
                        if ui.button("Translator") {
                            state.translator_window_open = !state.translator_window_open;
                        }

                        // Status text + player info on the right
                        let player_info_width = if let Some(ref pi) = state.player_info {
                            let name_realm = format!("{}  -  {}", pi.name, pi.realm);
                            let level_str = format!("  Lv.{}  ", pi.level);
                            let gold_str = format!("{}", pi.gold());
                            let silver_str = format!("{}", pi.silver());
                            let copper_str = format!("{}", pi.copper_rem());
                            ui.calc_text_size(&name_realm)[0]
                                + ui.calc_text_size(&level_str)[0]
                                + ui.calc_text_size(&gold_str)[0]
                                + ui.calc_text_size("g ")[0]
                                + ui.calc_text_size(&silver_str)[0]
                                + ui.calc_text_size("s ")[0]
                                + ui.calc_text_size(&copper_str)[0]
                                + ui.calc_text_size("c")[0]
                                + 8.0 // extra padding
                        } else {
                            0.0
                        };
                        let status_w = ui.calc_text_size(&state.status_text)[0];
                        let total_right = status_w + player_info_width + 24.0;
                        ui.same_line_with_pos(display_size[0] - total_right);
                        ui.text_colored([0.7, 0.7, 0.3, 1.0], &state.status_text);

                        if let Some(ref pi) = state.player_info {
                            ui.same_line();
                            ui.text_colored([1.0, 1.0, 1.0, 0.9], &format!("  {}  -  {}", pi.name, pi.realm));
                            ui.same_line();
                            ui.text_colored([0.5, 0.8, 1.0, 1.0], &format!("Lv.{}", pi.level));
                            ui.same_line();
                            // Gold (yellow)
                            ui.text_colored([1.0, 0.84, 0.0, 1.0], &format!("  {}", pi.gold()));
                            ui.same_line_with_spacing(0.0, 0.0);
                            ui.text_colored([0.85, 0.7, 0.0, 1.0], "g");
                            ui.same_line();
                            // Silver (light gray)
                            ui.text_colored([0.78, 0.78, 0.82, 1.0], &format!("{}", pi.silver()));
                            ui.same_line_with_spacing(0.0, 0.0);
                            ui.text_colored([0.6, 0.6, 0.65, 1.0], "s");
                            ui.same_line();
                            // Copper (brown/orange)
                            ui.text_colored([0.8, 0.5, 0.2, 1.0], &format!("{}", pi.copper_rem()));
                            ui.same_line_with_spacing(0.0, 0.0);
                            ui.text_colored([0.65, 0.4, 0.15, 1.0], "c");
                        }

                        appbar_hovered = ui.is_window_hovered();
                        appbar_height = ui.window_size()[1];
                    });

                // ── Dropdown panels ──────────────────────────────
                let mut dropdown_rect: Option<([f32; 2], [f32; 2])> = None;

                if let Some(dropdown) = state.open_dropdown {
                    let (label, width) = match dropdown {
                        AppBarDropdown::Process => ("##dropdown_process", 400.0_f32),
                        AppBarDropdown::Settings => ("##dropdown_settings", 420.0_f32),
                        AppBarDropdown::DebugTools => ("##dropdown_debug", 420.0_f32),
                    };

                    ui.window(label)
                        .position([10.0, appbar_height], imgui::Condition::Always)
                        .size([width, 0.0], imgui::Condition::Always)
                        .flags(
                            imgui::WindowFlags::NO_TITLE_BAR
                                | imgui::WindowFlags::NO_RESIZE
                                | imgui::WindowFlags::NO_MOVE
                                | imgui::WindowFlags::NO_COLLAPSE
                                | imgui::WindowFlags::NO_SAVED_SETTINGS
                                | imgui::WindowFlags::ALWAYS_AUTO_RESIZE,
                        )
                        .build(|| {
                            dropdown_rect = Some((ui.window_pos(), ui.window_size()));

                            match dropdown {
                                AppBarDropdown::Process => {
                                    ui.input_text("Process Name", &mut state.config.process_name)
                                        .build();

                                    ui.disabled(is_attached, || {
                                        if ui.button("Attach") {
                                            let mut sys = System::new();
                                            sys.refresh_processes(
                                                sysinfo::ProcessesToUpdate::All,
                                                true,
                                            );
                                            let found = sys.processes().values().find(|p| {
                                                p.name().to_string_lossy()
                                                    == state.config.process_name.as_str()
                                            });
                                            match found {
                                                Some(process) => {
                                                    let pid = process.pid().as_u32();
                                                    info!(
                                                        "Found process '{}' with PID={}",
                                                        state.config.process_name, pid
                                                    );
                                                    match state.reader.attach(pid) {
                                                        Ok(()) => {
                                                            state.attached_pid = Some(pid);
                                                            state.chat_reader.reset();
                                                            state.chat_messages.clear();
                                                            state.status_text = format!(
                                                                "Attached to {} (PID: {})",
                                                                state.config.process_name, pid
                                                            );
                                                            state.config.save();
                                                            info!(
                                                                "Successfully attached to PID={}",
                                                                pid
                                                            );
                                                        }
                                                        Err(e) => {
                                                            error!(
                                                                "Failed to attach to PID={}: {}",
                                                                pid, e
                                                            );
                                                            state.status_text =
                                                                format!("Failed to attach: {}", e);
                                                        }
                                                    }
                                                }
                                                None => {
                                                    warn!(
                                                        "Process '{}' not found",
                                                        state.config.process_name
                                                    );
                                                    state.status_text = format!(
                                                        "Process '{}' not found",
                                                        state.config.process_name
                                                    );
                                                }
                                            }
                                        }
                                    });

                                    ui.same_line();

                                    ui.disabled(!is_attached, || {
                                        if ui.button("Detach") {
                                            info!("User requested detach");
                                            if let Err(e) = state.reader.detach() {
                                                error!("Detach error: {}", e);
                                                state.status_text =
                                                    format!("Detach error: {}", e);
                                            } else {
                                                state.attached_pid = None;
                                                state.chat_reader.reset();
                                                state.status_text = String::from("Detached");
                                                info!("Detached successfully");
                                            }
                                        }
                                    });
                                }
                                AppBarDropdown::Settings => {
                                    // ── Appearance ───────────────────────
                                    ui.text("Appearance");
                                    ui.separator();

                                    // App language
                                    let langs = ["EN", "RU"];
                                    let mut lang_idx = langs
                                        .iter()
                                        .position(|l| *l == state.config.app_language.as_str())
                                        .unwrap_or(0);
                                    if ui.combo_simple_string("Language", &mut lang_idx, &langs) {
                                        state.config.app_language = langs[lang_idx].to_string();
                                        state.config.save();

                                    }

                                    // Font combo
                                    let font_labels: Vec<&str> = state
                                        .available_fonts
                                        .iter()
                                        .map(|f| f.name.as_str())
                                        .collect();
                                    let mut font_idx = state
                                        .available_fonts
                                        .iter()
                                        .position(|f| f.name == state.config.font_name)
                                        .unwrap_or(0);
                                    if ui.combo_simple_string("Font", &mut font_idx, &font_labels)
                                    {

                                        if font_idx < state.available_fonts.len() {
                                            state.config.font_name =
                                                state.available_fonts[font_idx].name.clone();
                                        }
                                    }

                                    // Font size
                                    let mut size = state.config.font_size;
                                    if ui
                                        .input_float("Font Size", &mut size)
                                        .step(1.0)
                                        .step_fast(4.0)
                                        .build()
                                    {
                                        state.config.font_size = size.clamp(10.0, 32.0);
                                    }

                                    // Theme combo
                                    let mut theme_idx = THEMES
                                        .iter()
                                        .position(|t| *t == state.config.theme.as_str())
                                        .unwrap_or(0);
                                    if ui.combo_simple_string("Theme", &mut theme_idx, THEMES) {

                                        state.config.theme = THEMES[theme_idx].to_string();
                                        state.theme_changed = true;
                                    }

                                    if ui.button("Apply") {
                                        state.font_changed = true;
                                        state.theme_changed = true;
                                        state.config.save();
                                    }

                                    ui.spacing();
                                    ui.spacing();

                                    // ── Game Configuration ───────────────
                                    ui.text("Game Configuration");
                                    ui.separator();

                                    ui.input_text(
                                        "WoW Folder",
                                        &mut state.config.wow_folder_path,
                                    )
                                    .build();

                                    if ui.button("Browse...") {
                                        let mut dialog = rfd::FileDialog::new();
                                        if !state.config.wow_folder_path.is_empty() {
                                            dialog = dialog
                                                .set_directory(&state.config.wow_folder_path);
                                        }
                                        if let Some(path) = dialog.pick_folder() {
                                            state.config.wow_folder_path =
                                                path.to_string_lossy().into_owned();
                                            state.config.save();
                                        }
                                    }

                                    ui.same_line();
                                    if ui.button("Scan")
                                        && !state.config.wow_folder_path.is_empty()
                                    {
                                        let path =
                                            std::path::Path::new(&state.config.wow_folder_path);
                                        match wtf_parser::find_character_configs(path) {
                                            Ok(configs) => {
                                                let count = configs.len();
                                                let saved = &state.config.selected_character;
                                                let idx = configs
                                                    .iter()
                                                    .position(|c| &c.display_label() == saved)
                                                    .unwrap_or(0);
                                                state.character_configs = configs;
                                                state.selected_char_index = idx;
                                                state.loaded_wtf_tabs = None;
                                                state.wtf_status =
                                                    format!("Found {} character(s)", count);
                                            }
                                            Err(e) => {
                                                state.character_configs.clear();
                                                state.loaded_wtf_tabs = None;
                                                state.wtf_status = format!("Scan error: {}", e);
                                            }
                                        }
                                    }

                                    if !state.character_configs.is_empty() {
                                        let labels: Vec<String> = state
                                            .character_configs
                                            .iter()
                                            .map(|c| c.display_label())
                                            .collect();
                                        let items: Vec<&str> =
                                            labels.iter().map(|s| s.as_str()).collect();
                                        if ui.combo_simple_string(
                                            "Character",
                                            &mut state.selected_char_index,
                                            &items,
                                        ) {
    
                                            state.loaded_wtf_tabs = None;
                                        }

                                        if ui.button("Load Config") {
                                            let cfg = &state.character_configs
                                                [state.selected_char_index];
                                            state.config.selected_character = cfg.display_label();
                                            state.config.save();
                                            match wtf_parser::parse_chat_cache(
                                                &cfg.chat_cache_path,
                                            ) {
                                                Ok(windows) => {
                                                    let tabs =
                                                        wtf_parser::to_chat_tabs(&windows);
                                                    state.wtf_status = format!(
                                                        "Loaded {} tabs from {}",
                                                        tabs.len(),
                                                        cfg.character,
                                                    );
                                                    state.loaded_wtf_tabs = Some(tabs);
                                                }
                                                Err(e) => {
                                                    state.wtf_status =
                                                        format!("Load error: {}", e);
                                                    state.loaded_wtf_tabs = None;
                                                }
                                            }
                                        }
                                    }

                                    if !state.wtf_status.is_empty() {
                                        ui.text_colored(
                                            [0.5, 0.7, 0.5, 1.0],
                                            &state.wtf_status,
                                        );
                                    }

                                    ui.spacing();
                                    ui.text_colored(
                                        [0.6, 0.6, 0.6, 1.0],
                                        "Tip: To import newly created chat tabs,",
                                    );
                                    ui.text_colored(
                                        [0.6, 0.6, 0.6, 1.0],
                                        "type /reload in game, then click Load Config",
                                    );
                                    ui.text_colored(
                                        [0.6, 0.6, 0.6, 1.0],
                                        "and Apply Filters.",
                                    );

                                    ui.spacing();
                                    ui.spacing();

                                    // ── Translation ─────────────────────────
                                    ui.text("Translation");
                                    ui.separator();

                                    ui.input_text("DeepL API Key", &mut state.api_key_input)
                                        .password(true)
                                        .build();

                                    // Target language dropdown
                                    if !state.target_languages.is_empty() {
                                        let lang_labels: Vec<String> = state
                                            .target_languages
                                            .iter()
                                            .map(|(code, name)| {
                                                format!("{} ({})", name, code)
                                            })
                                            .collect();
                                        let lang_items: Vec<&str> =
                                            lang_labels.iter().map(|s| s.as_str()).collect();
                                        let mut lang_idx = state
                                            .target_languages
                                            .iter()
                                            .position(|(code, _)| {
                                                code == &state.config.target_language
                                            })
                                            .unwrap_or(0);
                                        if ui.combo_simple_string(
                                            "Target Language",
                                            &mut lang_idx,
                                            &lang_items,
                                        ) {
    
                                            if lang_idx < state.target_languages.len() {
                                                state.config.target_language =
                                                    state.target_languages[lang_idx].0.clone();
                                            }
                                        }
                                    } else {
                                        ui.input_text(
                                            "Target Language",
                                            &mut state.config.target_language,
                                        )
                                        .build();
                                    }

                                    if ui.button("Save & Connect") {
                                        state.config.deepl_api_key =
                                            state.api_key_input.clone();
                                        state.config.save();
                                        state.translation_error.clear();
                                        state.translations.clear();

                                        // Shut down old service
                                        if let Some(ref svc) = state.translation_service {
                                            svc.shutdown();
                                        }
                                        state.translation_service = None;
                                        state.translation_rx = None;

                                        if !state.config.deepl_api_key.is_empty() {
                                            let (service, rx) =
                                                TranslationService::start(
                                                    state.config.deepl_api_key.clone(),
                                                    state.config.target_language.clone(),
                                                );
                                            service.fetch_languages();
                                            state.translation_service = Some(service);
                                            state.translation_rx = Some(rx);
                                        }
                                    }

                                    ui.same_line();
                                    let can_fetch =
                                        state.translation_service.is_some();
                                    ui.disabled(!can_fetch, || {
                                        if ui.button("Fetch Languages") {
                                            if let Some(ref svc) =
                                                state.translation_service
                                            {
                                                svc.fetch_languages();
                                            }
                                        }
                                    });

                                    // Status display
                                    if !state.translation_error.is_empty() {
                                        ui.text_colored(
                                            [1.0, 0.3, 0.3, 1.0],
                                            &state.translation_error,
                                        );
                                    } else if state.translation_service.is_some() {
                                        ui.text_colored(
                                            [0.3, 1.0, 0.3, 1.0],
                                            "Connected",
                                        );
                                    } else {
                                        ui.text_colored(
                                            [0.6, 0.6, 0.6, 1.0],
                                            "Not connected (enter API key)",
                                        );
                                    }
                                }
                                AppBarDropdown::DebugTools => {
                                    ui.text_wrapped(
                                        "Diagnostic tools for inspecting and locating \
                                         the WoW chat buffer in process memory.",
                                    );
                                    ui.separator();

                                    ui.text_wrapped(
                                        "Debug Scan reads all 60 chat buffer slots and \
                                         logs their raw fields.",
                                    );
                                    ui.disabled(!is_attached, || {
                                        if ui.button("Run Debug Scan") {
                                            info!("User requested debug scan");
                                            chat::debug_scan(&*state.reader);
                                            state.status_text =
                                                "Debug scan complete (see log)".into();
                                        }
                                    });

                                    ui.spacing();
                                    ui.separator();

                                    ui.text_wrapped(
                                        "Memory Scanner searches the entire process \
                                         address space for a given text string.",
                                    );
                                    ui.disabled(!is_attached, || {
                                        ui.input_text("Search Text", &mut state.search_text)
                                            .build();
                                        if ui.button("Scan Memory")
                                            && !state.search_text.is_empty()
                                        {
                                            info!(
                                                "Scanning memory for: \"{}\"",
                                                state.search_text
                                            );
                                            match state
                                                .reader
                                                .scan_for_bytes(state.search_text.as_bytes())
                                            {
                                                Ok(addrs) => {
                                                    if addrs.is_empty() {
                                                        state.status_text =
                                                            "Scan: no matches found".into();
                                                        warn!(
                                                            "No matches for \"{}\"",
                                                            state.search_text
                                                        );
                                                    } else {
                                                        state.status_text = format!(
                                                            "Scan: {} matches (see log)",
                                                            addrs.len()
                                                        );
                                                        chat::analyze_found_addresses(&addrs);
                                                    }
                                                }
                                                Err(e) => {
                                                    state.status_text =
                                                        format!("Scan error: {}", e);
                                                    error!("Scan error: {}", e);
                                                }
                                            }
                                        }
                                    });
                                }
                            }
                        });
                }

                // ── Close dropdown on outside click ──────────────
                let mouse = ui.io().mouse_pos;
                let mouse_in_dropdown = dropdown_rect.map_or(false, |(pos, size)| {
                    mouse[0] >= pos[0]
                        && mouse[0] <= pos[0] + size[0]
                        && mouse[1] >= pos[1]
                        && mouse[1] <= pos[1] + size[1]
                });
                if ui.is_mouse_clicked(imgui::MouseButton::Left)
                    && !appbar_hovered
                    && !mouse_in_dropdown
                {
                    state.open_dropdown = None;
                }

                // ── Window: Glossary Editor ──────────────────────
                if state.glossary_editor_open {
                    let mut still_open = true;
                    ui.window("Glossary Editor")
                        .size([500.0, 620.0], imgui::Condition::FirstUseEver)
                        .opened(&mut still_open)
                        .build(|| {
                            ui.text(&format!(
                                "{} entries in glossary",
                                state.glossary.entries.len()
                            ));
                            ui.separator();

                            // Scrollable list of entries
                            let list_height = ui.content_region_avail()[1] - 210.0;
                            if let Some(_child) = ui
                                .child_window("glossary_list")
                                .size([0.0, list_height.max(100.0)])
                                .border(true)
                                .begin()
                            {
                                let mut delete_idx: Option<usize> = None;
                                let mut edit_idx: Option<usize> = None;

                                for (i, entry) in state.glossary.entries.iter().enumerate() {
                                    let keys_str = entry.keys.join(", ");
                                    ui.text_colored([0.3, 0.9, 0.8, 1.0], &keys_str);
                                    if ui.is_item_hovered() {
                                        ui.tooltip(|| {
                                            let tooltip_width = 300.0_f32;
                                            let _wrap = ui.push_text_wrap_pos_with_pos(tooltip_width);
                                            let desc = match state.config.app_language.as_str() {
                                                "RU" if !entry.description_ru.is_empty() => &entry.description_ru,
                                                _ => &entry.description_en,
                                            };
                                            ui.text(desc);
                                            ui.dummy([tooltip_width, 0.0]);
                                        });
                                    }

                                    ui.same_line_with_pos(ui.content_region_avail()[0] - 80.0 + ui.cursor_pos()[0]);
                                    let edit_id = format!("Edit##{}", i);
                                    if ui.small_button(&edit_id) {
                                        edit_idx = Some(i);
                                    }
                                    ui.same_line();
                                    let del_id = format!("Del##{}", i);
                                    if ui.small_button(&del_id) {
                                        delete_idx = Some(i);
                                    }
                                }

                                // Handle edit button
                                if let Some(i) = edit_idx {
                                    state.glossary_edit_keys =
                                        state.glossary.entries[i].keys.join(", ");
                                    state.glossary_edit_description_en =
                                        state.glossary.entries[i].description_en.clone();
                                    state.glossary_edit_description_ru =
                                        state.glossary.entries[i].description_ru.clone();
                                    state.glossary_editing_index = Some(i);
                                    state.glossary_editor_status.clear();
                                }

                                // Handle delete button
                                if let Some(i) = delete_idx {
                                    state.glossary.entries.remove(i);
                                    state.glossary.rebuild_lookup();
                                    state.glossary.save();
                                    // Reset form if we were editing the deleted entry
                                    if state.glossary_editing_index == Some(i) {
                                        state.glossary_editing_index = None;
                                        state.glossary_edit_keys.clear();
                                        state.glossary_edit_description_en.clear();
                                        state.glossary_edit_description_ru.clear();
                                    } else if let Some(ref mut idx) = state.glossary_editing_index {
                                        if *idx > i {
                                            *idx -= 1;
                                        }
                                    }
                                    state.glossary_editor_status =
                                        "Entry deleted".to_string();
                                }
                            }

                            ui.separator();

                            // Add/Edit form
                            let form_label = if state.glossary_editing_index.is_some() {
                                "Editing entry"
                            } else {
                                "New entry"
                            };
                            ui.text(form_label);

                            ui.input_text("Keys (comma-separated)", &mut state.glossary_edit_keys)
                                .build();
                            ui.input_text_multiline(
                                "Description (EN)",
                                &mut state.glossary_edit_description_en,
                                [0.0, 40.0],
                            )
                            .build();
                            ui.input_text_multiline(
                                "Description (RU)",
                                &mut state.glossary_edit_description_ru,
                                [0.0, 40.0],
                            )
                            .build();

                            if ui.button("Save Entry") {
                                let keys: Vec<String> = state
                                    .glossary_edit_keys
                                    .split(',')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();

                                if keys.is_empty() {
                                    state.glossary_editor_status =
                                        "Error: at least one key is required".to_string();
                                } else if state.glossary_edit_description_en.trim().is_empty()
                                    && state.glossary_edit_description_ru.trim().is_empty()
                                {
                                    state.glossary_editor_status =
                                        "Error: at least one description is required".to_string();
                                } else {
                                    let entry = glossary::GlossaryEntry {
                                        keys,
                                        description_en: state
                                            .glossary_edit_description_en
                                            .trim()
                                            .to_string(),
                                        description_ru: state
                                            .glossary_edit_description_ru
                                            .trim()
                                            .to_string(),
                                    };

                                    if let Some(idx) = state.glossary_editing_index {
                                        state.glossary.entries[idx] = entry;
                                        state.glossary_editor_status =
                                            "Entry updated".to_string();
                                    } else {
                                        state.glossary.entries.push(entry);
                                        state.glossary_editor_status =
                                            "Entry added".to_string();
                                    }

                                    state.glossary.rebuild_lookup();
                                    state.glossary.save();
                                    state.glossary_editing_index = None;
                                    state.glossary_edit_keys.clear();
                                    state.glossary_edit_description_en.clear();
                                    state.glossary_edit_description_ru.clear();
                                }
                            }
                            ui.same_line();
                            if ui.button("Clear") {
                                state.glossary_editing_index = None;
                                state.glossary_edit_keys.clear();
                                state.glossary_edit_description_en.clear();
                                state.glossary_edit_description_ru.clear();
                                state.glossary_editor_status.clear();
                            }

                            if !state.glossary_editor_status.is_empty() {
                                let color = if state.glossary_editor_status.starts_with("Error") {
                                    [1.0, 0.3, 0.3, 1.0]
                                } else {
                                    [0.3, 1.0, 0.3, 1.0]
                                };
                                ui.text_colored(color, &state.glossary_editor_status);
                            }
                        });
                    if !still_open {
                        state.glossary_editor_open = false;
                    }
                }

                // ── Window: Translator ────────────────────────────
                if state.translator_window_open {
                    let mut still_open = true;
                    ui.window("Translator")
                        .size([500.0, 520.0], imgui::Condition::FirstUseEver)
                        .opened(&mut still_open)
                        .build(|| {
                            let avail_width = ui.content_region_avail()[0];

                            // Source language combo
                            if !state.target_languages.is_empty() {
                                // Build labels with "Auto-detect" prepended
                                let mut src_labels: Vec<String> = vec!["Auto-detect".into()];
                                src_labels.extend(
                                    state.target_languages.iter().map(|(code, name)| {
                                        format!("{} ({})", code, name)
                                    }),
                                );
                                let src_items: Vec<&str> =
                                    src_labels.iter().map(|s| s.as_str()).collect();

                                // Current selection: empty string = auto-detect (index 0)
                                let mut src_idx = if state.config.translator_source_lang.is_empty() {
                                    0
                                } else {
                                    state
                                        .target_languages
                                        .iter()
                                        .position(|(code, _)| {
                                            code == &state.config.translator_source_lang
                                        })
                                        .map(|i| i + 1) // offset by 1 for "Auto-detect"
                                        .unwrap_or(0)
                                };

                                if ui.combo_simple_string(
                                    "Source Language",
                                    &mut src_idx,
                                    &src_items,
                                ) {
                                    if src_idx == 0 {
                                        state.config.translator_source_lang = String::new();
                                    } else if src_idx - 1 < state.target_languages.len() {
                                        state.config.translator_source_lang =
                                            state.target_languages[src_idx - 1].0.clone();
                                    }
                                    state.config.save();
                                }
                            } else {
                                ui.input_text(
                                    "Source Language",
                                    &mut state.config.translator_source_lang,
                                )
                                .hint("empty = auto-detect")
                                .build();
                            }

                            // Target language combo
                            if !state.target_languages.is_empty() {
                                let tgt_labels: Vec<String> = state
                                    .target_languages
                                    .iter()
                                    .map(|(code, name)| format!("{} ({})", code, name))
                                    .collect();
                                let tgt_items: Vec<&str> =
                                    tgt_labels.iter().map(|s| s.as_str()).collect();
                                let mut tgt_idx = state
                                    .target_languages
                                    .iter()
                                    .position(|(code, _)| {
                                        code == &state.config.translator_target_lang
                                    })
                                    .unwrap_or(0);
                                if ui.combo_simple_string(
                                    "Target Language##translator",
                                    &mut tgt_idx,
                                    &tgt_items,
                                ) {
                                    if tgt_idx < state.target_languages.len() {
                                        state.config.translator_target_lang =
                                            state.target_languages[tgt_idx].0.clone();
                                    }
                                    state.config.save();
                                }
                            } else {
                                ui.input_text(
                                    "Target Language##translator",
                                    &mut state.config.translator_target_lang,
                                )
                                .build();
                            }

                            ui.separator();

                            // Input text area
                            ui.input_text_multiline(
                                "##translator_input",
                                &mut state.translator_input,
                                [avail_width, 120.0],
                            )
                            .build();

                            // Translate button
                            let can_translate = !state.translator_pending
                                && !state.translator_input.trim().is_empty()
                                && state.translation_service.is_some();
                            ui.disabled(!can_translate, || {
                                if ui.button("Translate") {
                                    state.translator_pending = true;
                                    state.translator_error.clear();
                                    state.translator_output.clear();

                                    // Resolve effective source lang:
                                    // If translator_source_lang is empty, fall back to config.target_language
                                    let src = if state.config.translator_source_lang.is_empty() {
                                        Some(state.config.target_language.clone())
                                    } else {
                                        Some(state.config.translator_source_lang.clone())
                                    };

                                    if let Some(ref service) = state.translation_service {
                                        service.translate(TranslationRequest {
                                            message_id: u64::MAX,
                                            text: state.translator_input.clone(),
                                            link_names: Vec::new(),
                                            source_lang: src,
                                            target_lang: Some(
                                                state.config.translator_target_lang.clone(),
                                            ),
                                        });
                                    }
                                }
                            });

                            if state.translator_pending {
                                ui.same_line();
                                ui.text_disabled("Translating...");
                            }

                            ui.same_line();

                            // Swap Languages button
                            if ui.button("Swap Languages") {
                                std::mem::swap(
                                    &mut state.config.translator_source_lang,
                                    &mut state.config.translator_target_lang,
                                );
                                state.config.save();
                            }

                            ui.separator();

                            // Output text area (read-only)
                            ui.input_text_multiline(
                                "##translator_output",
                                &mut state.translator_output,
                                [avail_width, 120.0],
                            )
                            .read_only(true)
                            .build();

                            // Copy Result button
                            if ui.button("Copy Result") && !state.translator_output.is_empty() {
                                if let Some(ref mut cb) = state.clipboard {
                                    cb.copy(&state.translator_output);
                                }
                            }

                            // Error display
                            if !state.translator_error.is_empty() {
                                ui.text_colored(
                                    [1.0, 0.3, 0.3, 1.0],
                                    &format!("Error: {}", state.translator_error),
                                );
                            }

                            // Service status
                            if state.translation_service.is_none() {
                                ui.text_colored(
                                    [0.6, 0.6, 0.6, 1.0],
                                    "Translation service not connected. Configure API key in Settings.",
                                );
                            }
                        });
                    if !still_open {
                        state.translator_window_open = false;
                    }
                }

                // ── Window: Chat ─────────────────────────────────
                ui.window("Chat")
                    .size([1080.0, 700.0], imgui::Condition::FirstUseEver)
                    .position([10.0, 45.0], imgui::Condition::FirstUseEver)
                    .build(|| {
                        if ui.button("Clear") {
                            state.chat_messages.clear();
                        }

                        ui.same_line();
                        if ui.button("Copy All") {
                            if let Some(ref mut cb) = state.clipboard {
                                let active_tab = &state.chat_tabs[state.active_tab];
                                let text: String = state
                                    .chat_messages
                                    .iter()
                                    .filter(|m| active_tab.matches(m.message_type))
                                    .map(|m| m.display_line())
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                cb.copy(&text);
                            }
                        }

                        ui.same_line();
                        ui.disabled(state.loaded_wtf_tabs.is_none(), || {
                            if ui.button("Apply Filters") {
                                if let Some(tabs) = state.loaded_wtf_tabs.take() {
                                    state.chat_tabs = tabs;
                                    state.active_tab = 0;
                                }
                            }
                        });

                        ui.same_line();
                        if ui.button("Reset Filters") {
                            state.chat_tabs = chat::default_tabs();
                            state.active_tab = 0;
                        }

                        ui.same_line();
                        if ui.checkbox("Translate Always", &mut state.auto_translate) {
                            state.config.auto_translate = state.auto_translate;
                            state.config.save();
                        }

                        // Translation error warning bar
                        if !state.translation_error.is_empty() {
                            ui.text_colored(
                                [1.0, 0.8, 0.2, 1.0],
                                &format!("Translation: {}", state.translation_error),
                            );
                        }

                        ui.separator();

                        let mut translate_requests: Vec<(u64, Vec<TextSegment>)> = Vec::new();
                        if let Some(_tab_bar) = ui.tab_bar("chat_tabs") {
                            for (tab_idx, tab) in state.chat_tabs.iter().enumerate() {
                                if let Some(_tab_item) = ui.tab_item(&tab.name) {
                                    state.active_tab = tab_idx;
                                    render_chat_area(
                                        ui,
                                        &state.chat_messages,
                                        tab,
                                        tab_idx,
                                        state.had_new_messages,
                                        &mut state.clipboard,
                                        &state.translations,
                                        state.translation_service.is_some(),
                                        &mut translate_requests,
                                        &state.glossary,
                                        &state.config.app_language,
                                    );
                                }
                            }
                        }
                        // Process any translation requests from [T] button clicks
                        if let Some(ref service) = state.translation_service {
                            for (msg_id, segments) in translate_requests {
                                let (text, link_names) =
                                    translation::prepare_for_translation(&segments);
                                if !text.trim().is_empty() {
                                    state
                                        .translations
                                        .insert(msg_id, TranslationEntry::Pending);
                                    service.translate(TranslationRequest {
                                        message_id: msg_id,
                                        text,
                                        link_names,
                                        source_lang: None,
                                        target_lang: None,
                                    });
                                }
                            }
                        }
                    });

                let draw_data = imgui.render();

                unsafe {
                    glow_ctx.clear_color(0.1, 0.1, 0.1, 1.0);
                    glow_ctx.clear(glow::COLOR_BUFFER_BIT);
                }

                renderer.render(draw_data).expect("Failed to render");

                gl_surface
                    .swap_buffers(gl_context)
                    .expect("Failed to swap buffers");

                window.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

// ─── URL opener ──────────────────────────────────────────────────────

fn open_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

// ─── Glossary-aware text renderer ────────────────────────────────────

/// Render a plain text segment with per-word glossary highlighting.
/// Matched words are tinted toward teal and show a tooltip on hover.
/// Returns true if the last rendered item was hovered.
fn render_plain_with_glossary(
    ui: &imgui::Ui,
    text: &str,
    msg_color: [f32; 4],
    glossary: &glossary::Glossary,
    lang: &str,
    needs_same_line: bool,
) -> bool {
    let mut hovered = false;

    // Fast path: empty glossary
    if glossary.is_empty() {
        if needs_same_line {
            ui.same_line_with_spacing(0.0, 0.0);
        }
        ui.text_colored(msg_color, text);
        return ui.is_item_hovered();
    }

    // Tokenize and check for any matches
    let tokens = glossary::tokenize(text);
    let has_match = tokens
        .iter()
        .any(|(tok, is_word)| *is_word && glossary.lookup_word(tok, lang).is_some());

    if !has_match {
        // No matches fast path
        if needs_same_line {
            ui.same_line_with_spacing(0.0, 0.0);
        }
        ui.text_colored(msg_color, text);
        return ui.is_item_hovered();
    }

    // Render token-by-token
    let mut first = !needs_same_line;
    for (tok, is_word) in &tokens {
        if first {
            first = false;
        } else {
            // Check if token fits on the current line before joining
            let prev_end_x = ui.item_rect_max()[0];
            let tok_w = ui.calc_text_size(tok)[0];
            let content_right_x = ui.cursor_screen_pos()[0] + ui.content_region_avail()[0];
            if prev_end_x + tok_w <= content_right_x {
                ui.same_line_with_spacing(0.0, 0.0);
            }
        }

        if *is_word {
            if let Some(description) = glossary.lookup_word(tok, lang) {
                // Glossary match: tint toward teal/cyan
                let teal_color = [
                    msg_color[0] * 0.5,
                    msg_color[1] * 0.5 + 0.45,
                    msg_color[2] * 0.5 + 0.4,
                    msg_color[3],
                ];
                ui.text_colored(teal_color, tok);
                if ui.is_item_hovered() {
                    hovered = true;
                    ui.tooltip(|| {
                        let tooltip_width = 300.0_f32;
                        let _wrap = ui.push_text_wrap_pos_with_pos(tooltip_width);
                        ui.text_colored([1.0, 0.9, 0.5, 1.0], tok);
                        ui.separator();
                        ui.text(description);
                        ui.dummy([tooltip_width, 0.0]);
                    });
                }
            } else {
                ui.text_colored(msg_color, tok);
                if ui.is_item_hovered() {
                    hovered = true;
                }
            }
        } else {
            ui.text_colored(msg_color, tok);
            if ui.is_item_hovered() {
                hovered = true;
            }
        }
    }

    hovered
}

// ─── Chat area renderer ─────────────────────────────────────────────

fn render_chat_area(
    ui: &imgui::Ui,
    messages: &[ChatMessage],
    tab: &ChatTab,
    tab_idx: usize,
    had_new_messages: bool,
    clipboard: &mut Option<clipboard::ClipboardHelper>,
    translations: &HashMap<u64, TranslationEntry>,
    has_translation_service: bool,
    translate_requests: &mut Vec<(u64, Vec<TextSegment>)>,
    glossary: &glossary::Glossary,
    app_language: &str,
) {
    let id = format!("chat_area_{}", tab_idx);
    let child_size = [0.0, -1.0f32];

    if let Some(_child) = ui
        .child_window(&id)
        .size(child_size)
        .border(true)
        .begin()
    {
        let _wrap = ui.push_text_wrap_pos_with_pos(0.0);

        let filtered: Vec<&ChatMessage> = messages
            .iter()
            .filter(|m| tab.matches(m.message_type))
            .collect();

        if filtered.is_empty() {
            ui.text_disabled("No messages yet. Attach to a process to begin reading chat.");
        } else {
            for (index, msg) in filtered.iter().enumerate() {
                let msg_color = msg.message_type.color();
                let line = msg.display_line();
                let popup_id = format!("msg_ctx_{}_{}", tab_idx, index);
                let mut line_hovered = false;

                // [T] translate button
                let entry = translations.get(&msg.id);
                let btn_id = format!("T##t_{}_{}", tab_idx, index);
                if let Some(TranslationEntry::Pending) = entry {
                    ui.text_disabled("[...]");
                } else if has_translation_service {
                    if ui.small_button(&btn_id) {
                        translate_requests.push((msg.id, msg.segments.clone()));
                    }
                }
                ui.same_line();

                if msg.has_links() {
                    // Rich rendering: prefix + inline colored segments
                    let prefix = msg.display_prefix();
                    ui.text_colored(msg_color, &prefix);
                    if ui.is_item_hovered() {
                        line_hovered = true;
                    }

                    for seg in &msg.segments {
                        match seg {
                            TextSegment::Plain(text) => {
                                if render_plain_with_glossary(
                                    ui, text, msg_color, glossary, app_language, true,
                                ) {
                                    line_hovered = true;
                                }
                            }
                            TextSegment::WowLink {
                                link_type,
                                display_name,
                                color,
                            } => {
                                let prev_end_x = ui.item_rect_max()[0];
                                let tok_w = ui.calc_text_size(display_name)[0];
                                let content_right_x = ui.cursor_screen_pos()[0]
                                    + ui.content_region_avail()[0];
                                if prev_end_x + tok_w <= content_right_x {
                                    ui.same_line_with_spacing(0.0, 0.0);
                                }
                                ui.text_colored(*color, display_name);
                                if ui.is_item_hovered() {
                                    line_hovered = true;
                                    let url = link_type.wowhead_url(display_name);
                                    ui.tooltip_text(&url);
                                    if ui.is_mouse_clicked(imgui::MouseButton::Left) {
                                        open_url(&url);
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Simple rendering with glossary highlights
                    let prefix = msg.display_prefix();
                    ui.text_colored(msg_color, &prefix);
                    if ui.is_item_hovered() {
                        line_hovered = true;
                    }
                    if render_plain_with_glossary(
                        ui, &msg.text, msg_color, glossary, app_language, true,
                    ) {
                        line_hovered = true;
                    }
                }

                // Right-click context menu (works for both simple and rich rendering)
                if line_hovered && ui.is_mouse_released(imgui::MouseButton::Right) {
                    ui.open_popup(&popup_id);
                }
                if let Some(_popup) = ui.begin_popup(&popup_id) {
                    if ui.selectable("Copy") {
                        if let Some(ref mut cb) = clipboard {
                            cb.copy(&line);
                        }
                    }
                    if ui.selectable("Copy Text Only") {
                        if let Some(ref mut cb) = clipboard {
                            cb.copy(&msg.text);
                        }
                    }
                }

                // Show translation result below the message
                match entry {
                    Some(TranslationEntry::Done(translated)) => {
                        ui.text_colored(
                            [0.6, 0.8, 0.6, 1.0],
                            &format!("  \u{21B3} {}", translated),
                        );
                    }
                    Some(TranslationEntry::Error(err)) => {
                        ui.text_colored(
                            [1.0, 0.3, 0.3, 1.0],
                            &format!("  \u{21B3} Translation error: {}", err),
                        );
                    }
                    _ => {}
                }
            }

            if had_new_messages {
                ui.set_scroll_here_y();
            }
        }
    }
}

// ─── Logging & history helpers ────────────────────────────────────────

const MAX_LOG_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

fn rotate_file(path: &std::path::Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() >= MAX_LOG_SIZE {
            let old = path.with_extension("old");
            let _ = std::fs::rename(path, old);
        }
    }
}

fn setup_logging() {
    let log_path = config::config_dir().join("wotlk.log");
    rotate_file(&log_path);

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);

    let mut dispatch = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339_millis(std::time::SystemTime::now()),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Info)
        .chain(std::io::stderr());

    if let Ok(file) = log_file {
        dispatch = dispatch.chain(file);
    } else {
        eprintln!("Warning: could not open log file {}", log_path.display());
    }

    dispatch.apply().expect("Failed to initialize logger");
}

fn append_chat_history(messages: &[ChatMessage]) {
    let history_path = config::config_dir().join("chat.history");
    rotate_file(&history_path);

    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&history_path)
    else {
        error!("Failed to open chat history file");
        return;
    };

    use std::io::Write;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    for msg in messages {
        let _ = writeln!(file, "[{}] {}", now, msg.display_line());
    }
}

// ─── Main ────────────────────────────────────────────────────────────

fn main() {
    setup_logging();

    info!("WotLK Chat Translator starting");
    info!(
        "Chat buffer: start=0x{:X} stride=0x{:X} count_addr=0x{:X} slots={}",
        offsets::CHAT_BUFFER_START,
        offsets::CHAT_MESSAGE_STRIDE,
        offsets::CHAT_BUFFER_COUNT,
        offsets::CHAT_BUFFER_SIZE,
    );

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("Event loop error");
}
