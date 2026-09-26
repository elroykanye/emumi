use crate::{
    android::AndroidTools,
    config::AppConfig,
    model::{
        AndroidProfile, HostStats, LogEntry, LogLevel, PicturePreset, ProfileOptions, SpeedPreset,
        WindowPreset,
    },
    monitor::HostMonitor,
};
use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Frame, Layout, Margin, RichText, Stroke, Vec2,
};
use std::time::{Duration, Instant, SystemTime};

const BG: Color32 = Color32::from_rgb(244, 242, 236);
const PANEL: Color32 = Color32::from_rgb(253, 252, 249);
const INK: Color32 = Color32::from_rgb(31, 34, 31);
const MUTED: Color32 = Color32::from_rgb(111, 115, 109);
const ACCENT: Color32 = Color32::from_rgb(67, 139, 96);
const ACCENT_SOFT: Color32 = Color32::from_rgb(222, 238, 227);
const BORDER: Color32 = Color32::from_rgb(222, 220, 213);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Androids,
    Monitor,
    Logs,
    Settings,
}

pub struct EmuMiApp {
    page: Page,
    config: AppConfig,
    tools: AndroidTools,
    profiles: Vec<AndroidProfile>,
    selected: Option<usize>,
    logs: Vec<LogEntry>,
    monitor: HostMonitor,
    stats: HostStats,
    last_sample: Instant,
}

impl EmuMiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        let mut config = AppConfig::load();
        let tools = AndroidTools::detect(&config.android_sdk_path, &config.jdk_path);
        if config.android_sdk_path.is_empty() {
            config.android_sdk_path = tools
                .sdk_root
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
        }
        if config.jdk_path.is_empty() {
            config.jdk_path = tools
                .java_home
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
        }
        let profiles = tools.discover_profiles();
        let mut app = Self {
            page: Page::Androids,
            config,
            tools,
            selected: (!profiles.is_empty()).then_some(0),
            profiles,
            logs: Vec::new(),
            monitor: HostMonitor::default(),
            stats: HostStats::default(),
            last_sample: Instant::now() - Duration::from_secs(3),
        };
        app.log(LogLevel::Success, "EmuMi is ready");
        if app.tools.emulator.is_none() || app.tools.adb.is_none() {
            app.log(
                LogLevel::Warning,
                "Android tools need attention; open Settings for details",
            );
        }
        app
    }

    fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.logs.push(LogEntry {
            at: SystemTime::now(),
            level,
            message: message.into(),
        });
    }

    fn refresh(&mut self) {
        self.tools = AndroidTools::detect(&self.config.android_sdk_path, &self.config.jdk_path);
        self.profiles = self.tools.discover_profiles();
        self.selected = match (self.selected, self.profiles.is_empty()) {
            (_, true) => None,
            (Some(index), false) => Some(index.min(self.profiles.len() - 1)),
            (None, false) => Some(0),
        };
        self.log(
            LogLevel::Info,
            format!("Found {} Android profile(s)", self.profiles.len()),
        );
    }

    fn stop_all(&mut self) {
        let targets = self
            .profiles
            .iter()
            .filter_map(|p| p.running_serial.clone())
            .collect::<Vec<_>>();
        if targets.is_empty() {
            self.log(LogLevel::Info, "No Androids are running");
            return;
        }
        for serial in targets {
            match self.tools.stop(&serial) {
                Ok(()) => self.log(LogLevel::Success, format!("Stopped {serial}")),
                Err(err) => self.log(LogLevel::Error, format!("Could not stop {serial}: {err}")),
            }
        }
        self.refresh();
    }

    fn launch_args(options: &ProfileOptions) -> Vec<String> {
        let mut args = vec![
            "-cores".into(),
            options.cores.to_string(),
            "-memory".into(),
            options.memory_mb.to_string(),
            "-dpi-device".into(),
            options.dpi.to_string(),
            "-gpu".into(),
            options.gpu_mode.clone(),
        ];
        if let Some(port) = options.adb_port {
            args.extend(["-port".into(), port.to_string()]);
        }
        if options.cold_boot {
            args.push("-no-snapshot-load".into());
        }
        args
    }

    fn top_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("top_bar")
            .exact_size(68.0)
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(24, 14)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("EmuMi").size(25.0).strong().color(INK));
                    ui.label(RichText::new("Android emulators, made easy.").color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_sized(
                                [132.0, 38.0],
                                egui::Button::new(
                                    RichText::new("＋ New Android").color(Color32::WHITE),
                                )
                                .fill(ACCENT)
                                .stroke(Stroke::NONE)
                                .corner_radius(9),
                            )
                            .clicked()
                        {
                            self.log(LogLevel::Info, "Profile creation is next on the roadmap");
                        }
                        if ui
                            .add_sized(
                                [92.0, 38.0],
                                egui::Button::new("Stop all")
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::new(1.0, BORDER))
                                    .corner_radius(9),
                            )
                            .clicked()
                        {
                            self.stop_all();
                        }
                    });
                });
            });
    }

    fn sidebar(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("sidebar")
            .exact_size(190.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(238, 236, 229))
                    .inner_margin(Margin::symmetric(14, 20)),
            )
            .show(root, |ui| {
                ui.label(RichText::new("YOUR SPACE").size(10.0).strong().color(MUTED));
                ui.add_space(8.0);
                nav_button(ui, &mut self.page, Page::Androids, "▣  Androids");
                nav_button(ui, &mut self.page, Page::Monitor, "⌁  Monitor");
                nav_button(ui, &mut self.page, Page::Logs, "≡  Logs");
                nav_button(ui, &mut self.page, Page::Settings, "⚙  Settings");
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    let ready = self.tools.emulator.is_some() && self.tools.adb.is_some();
                    ui.label(
                        RichText::new(if ready {
                            "●  Android tools ready"
                        } else {
                            "●  Setup needed"
                        })
                        .size(12.0)
                        .color(if ready {
                            ACCENT
                        } else {
                            Color32::from_rgb(191, 119, 49)
                        }),
                    );
                });
            });
    }

    fn androids_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Androids").color(INK));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("↻  Refresh").clicked() {
                    self.refresh();
                }
            });
        });
        ui.label(RichText::new("Pick a profile, press start, and get on with it.").color(MUTED));
        ui.add_space(18.0);

        ui.columns(2, |columns| {
            columns[0].set_width(330.0);
            Frame::new().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).corner_radius(12).inner_margin(12).show(&mut columns[0], |ui| {
                ui.label(RichText::new(format!("{} PROFILES", self.profiles.len())).size(10.0).strong().color(MUTED));
                ui.add_space(8.0);
                if self.profiles.is_empty() {
                    ui.label(RichText::new("No Android profiles found").strong());
                    ui.label(RichText::new("Open Settings to check your Android SDK path.").color(MUTED));
                }
                for (index, profile) in self.profiles.iter().enumerate() {
                    let selected = self.selected == Some(index);
                    let response = Frame::new().fill(if selected { ACCENT_SOFT } else { Color32::TRANSPARENT }).corner_radius(9).inner_margin(10).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("▰").size(22.0).color(if profile.is_running() { ACCENT } else { MUTED }));
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&profile.name).strong().color(INK));
                                ui.label(RichText::new(profile.subtitle()).size(11.0).color(MUTED));
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if profile.is_running() { ui.label(RichText::new("RUNNING").size(9.0).strong().color(ACCENT)); }
                            });
                        });
                    }).response.interact(egui::Sense::click());
                    if response.clicked() { self.selected = Some(index); }
                    ui.add_space(4.0);
                }
            });

            columns[1].add_space(8.0);
            if let Some(index) = self.selected.filter(|i| *i < self.profiles.len()) {
                self.profile_detail(&mut columns[1], index);
            } else {
                Frame::new().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).corner_radius(12).inner_margin(24).show(&mut columns[1], |ui| {
                    ui.heading("Your Android will appear here");
                    ui.label(RichText::new("Once an AVD exists, EmuMi keeps its everyday controls in one calm place.").color(MUTED));
                });
            }
        });
    }

    fn profile_detail(&mut self, ui: &mut egui::Ui, index: usize) {
        let profile = self.profiles[index].clone();
        let options = self
            .config
            .profile_options
            .entry(profile.name.clone())
            .or_default();
        let mut action: Option<bool> = None;
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(12)
            .inner_margin(22)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.heading(RichText::new(&profile.name).color(INK));
                        ui.label(RichText::new(profile.subtitle()).color(MUTED));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let label = if profile.is_running() {
                            "■  Stop"
                        } else {
                            "▶  Start"
                        };
                        if ui
                            .add_sized(
                                [100.0, 40.0],
                                egui::Button::new(
                                    RichText::new(label).color(Color32::WHITE).strong(),
                                )
                                .fill(ACCENT)
                                .stroke(Stroke::NONE)
                                .corner_radius(9),
                            )
                            .clicked()
                        {
                            action = Some(profile.is_running());
                        }
                    });
                });
                ui.add_space(22.0);
                ui.label(
                    RichText::new("SIMPLE SETTINGS")
                        .size(10.0)
                        .strong()
                        .color(MUTED),
                );
                ui.add_space(8.0);
                preset_row(
                    ui,
                    "Speed",
                    "How much of your computer it may use",
                    &mut options.speed,
                    SpeedPreset::ALL,
                    SpeedPreset::label,
                );
                preset_row(
                    ui,
                    "Picture",
                    "A comfortable resolution and density",
                    &mut options.picture,
                    PicturePreset::ALL,
                    PicturePreset::label,
                );
                preset_row(
                    ui,
                    "Window",
                    "How the emulator opens",
                    &mut options.window,
                    WindowPreset::ALL,
                    WindowPreset::label,
                );
                ui.add_space(12.0);
                egui::CollapsingHeader::new(RichText::new("Advanced settings").strong())
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::Grid::new("advanced_grid")
                            .num_columns(2)
                            .spacing([18.0, 10.0])
                            .show(ui, |ui| {
                                ui.label("CPU cores");
                                ui.add(egui::Slider::new(&mut options.cores, 1..=16));
                                ui.end_row();
                                ui.label("Memory");
                                ui.add(
                                    egui::Slider::new(&mut options.memory_mb, 512..=16384)
                                        .suffix(" MB"),
                                );
                                ui.end_row();
                                ui.label("Display density");
                                ui.add(
                                    egui::Slider::new(&mut options.dpi, 120..=640).suffix(" dpi"),
                                );
                                ui.end_row();
                                ui.label("ADB console port");
                                let mut port = options.adb_port.unwrap_or(0);
                                if ui
                                    .add(egui::DragValue::new(&mut port).range(0..=65535))
                                    .changed()
                                {
                                    options.adb_port = (port != 0).then_some(port);
                                }
                                ui.end_row();
                                ui.label("Graphics");
                                egui::ComboBox::from_id_salt("gpu_mode")
                                    .selected_text(&options.gpu_mode)
                                    .show_ui(ui, |ui| {
                                        for mode in ["auto", "host", "swiftshader_indirect"] {
                                            ui.selectable_value(
                                                &mut options.gpu_mode,
                                                mode.to_owned(),
                                                mode,
                                            );
                                        }
                                    });
                                ui.end_row();
                                ui.label("Boot");
                                ui.checkbox(&mut options.cold_boot, "Cold boot next time");
                                ui.end_row();
                            });
                    });
            });
        if let Some(was_running) = action {
            if was_running {
                let serial = profile.running_serial.as_deref().unwrap_or_default();
                match self.tools.stop(serial) {
                    Ok(()) => self.log(LogLevel::Success, format!("Stopping {}", profile.name)),
                    Err(err) => self.log(
                        LogLevel::Error,
                        format!("Could not stop {}: {err}", profile.name),
                    ),
                }
            } else {
                let args = Self::launch_args(options);
                match self.tools.start(&profile, &args) {
                    Ok(()) => self.log(LogLevel::Success, format!("Starting {}", profile.name)),
                    Err(err) => self.log(
                        LogLevel::Error,
                        format!("Could not start {}: {err}", profile.name),
                    ),
                }
            }
        }
    }

    fn monitor_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Monitor").color(INK));
        ui.label(RichText::new("A quick view of the host and every running Android.").color(MUTED));
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            metric_card(
                ui,
                "HOST CPU",
                &format!("{:.0}%", self.stats.cpu_percent),
                self.stats.cpu_percent / 100.0,
            );
            let memory_percent = if self.stats.memory_total_bytes == 0 {
                0.0
            } else {
                self.stats.memory_used_bytes as f32 / self.stats.memory_total_bytes as f32
            };
            metric_card(
                ui,
                "HOST MEMORY",
                &format!(
                    "{:.1} / {:.1} GB",
                    gib(self.stats.memory_used_bytes),
                    gib(self.stats.memory_total_bytes)
                ),
                memory_percent,
            );
            let running = self.profiles.iter().filter(|p| p.is_running()).count();
            metric_card(
                ui,
                "RUNNING",
                &format!("{running} Androids"),
                if self.profiles.is_empty() {
                    0.0
                } else {
                    running as f32 / self.profiles.len() as f32
                },
            );
        });
        ui.add_space(18.0);
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(12)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("LIVE ANDROIDS")
                        .size(10.0)
                        .strong()
                        .color(MUTED),
                );
                ui.add_space(8.0);
                let mut any = false;
                for profile in self.profiles.iter().filter(|p| p.is_running()) {
                    any = true;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").color(ACCENT));
                        ui.label(RichText::new(&profile.name).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(profile.running_serial.as_deref().unwrap_or("Starting…"));
                        });
                    });
                    ui.separator();
                }
                if !any {
                    ui.label(RichText::new("Nothing is running yet.").color(MUTED));
                }
            });
    }

    fn logs_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Logs").color(INK));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Clear").clicked() {
                    self.logs.clear();
                }
            });
        });
        ui.label(
            RichText::new("Human-readable activity first; raw emulator output stays on disk.")
                .color(MUTED),
        );
        ui.add_space(18.0);
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(12)
            .inner_margin(14)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.logs.is_empty() {
                        ui.label(RichText::new("No activity yet.").color(MUTED));
                    }
                    for entry in self.logs.iter().rev() {
                        ui.horizontal(|ui| {
                            let (dot, color) = match entry.level {
                                LogLevel::Info => ("●", Color32::from_rgb(76, 126, 171)),
                                LogLevel::Success => ("●", ACCENT),
                                LogLevel::Warning => ("●", Color32::from_rgb(191, 119, 49)),
                                LogLevel::Error => ("●", Color32::from_rgb(184, 72, 65)),
                            };
                            ui.label(RichText::new(dot).color(color));
                            ui.label(RichText::new(&entry.message).color(INK));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(relative_time(entry.at))
                                        .size(11.0)
                                        .color(MUTED),
                                );
                            });
                        });
                        ui.separator();
                    }
                });
            });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Settings").color(INK));
        ui.label(RichText::new("Tell EmuMi where your Android and Java tools live.").color(MUTED));
        ui.add_space(18.0);
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(12)
            .inner_margin(20)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("DEVELOPER TOOLS")
                        .size(10.0)
                        .strong()
                        .color(MUTED),
                );
                ui.add_space(12.0);
                path_field(
                    ui,
                    "Android SDK",
                    "Contains emulator and platform-tools",
                    &mut self.config.android_sdk_path,
                    self.tools.emulator.is_some() && self.tools.adb.is_some(),
                );
                ui.add_space(14.0);
                path_field(
                    ui,
                    "JDK",
                    "Java home used by Android command-line tools",
                    &mut self.config.jdk_path,
                    self.tools.java_home.is_some(),
                );
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_sized(
                            [118.0, 36.0],
                            egui::Button::new(RichText::new("Save settings").color(Color32::WHITE))
                                .fill(ACCENT)
                                .stroke(Stroke::NONE)
                                .corner_radius(8),
                        )
                        .clicked()
                    {
                        match self.config.save() {
                            Ok(path) => {
                                self.tools = AndroidTools::detect(
                                    &self.config.android_sdk_path,
                                    &self.config.jdk_path,
                                );
                                self.log(
                                    LogLevel::Success,
                                    format!("Settings saved to {}", path.display()),
                                );
                            }
                            Err(err) => {
                                self.log(LogLevel::Error, format!("Could not save settings: {err}"))
                            }
                        }
                    }
                    if ui.button("Auto-detect again").clicked() {
                        let detected = AndroidTools::detect("", "");
                        self.config.android_sdk_path = detected
                            .sdk_root
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        self.config.jdk_path = detected
                            .java_home
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        self.tools = detected;
                        self.log(LogLevel::Info, "Developer tools auto-detected");
                    }
                });
            });
        ui.add_space(16.0);
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(12)
            .inner_margin(20)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("SETUP HEALTH")
                        .size(10.0)
                        .strong()
                        .color(MUTED),
                );
                ui.add_space(10.0);
                health_row(
                    ui,
                    "Android Emulator",
                    self.tools.emulator.is_some(),
                    "Required to open profiles",
                );
                health_row(
                    ui,
                    "ADB",
                    self.tools.adb.is_some(),
                    "Connects EmuMi to running Androids",
                );
                health_row(
                    ui,
                    "AVD Manager",
                    self.tools.avd_manager.is_some(),
                    "Needed to create and edit profiles",
                );
                health_row(
                    ui,
                    "KVM acceleration",
                    self.tools.kvm_available,
                    "Makes emulation fast on Linux",
                );
                health_row(
                    ui,
                    "Java",
                    self.tools.java_home.is_some(),
                    "Needed by SDK management commands",
                );
            });
    }
}

impl eframe::App for EmuMiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.last_sample.elapsed() >= Duration::from_secs(2) {
            self.stats = self.monitor.sample();
            self.last_sample = Instant::now();
        }
        ui.ctx().request_repaint_after(Duration::from_millis(500));
        self.top_bar(ui);
        self.sidebar(ui);
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(26, 24)),
            )
            .show(ui, |ui| match self.page {
                Page::Androids => self.androids_page(ui),
                Page::Monitor => self.monitor_page(ui),
                Page::Logs => self.logs_page(ui),
                Page::Settings => self.settings_page(ui),
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.config.save();
    }
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = BG;
    style.visuals.widgets.inactive.fg_stroke.color = INK;
    style.visuals.widgets.hovered.bg_fill = ACCENT_SOFT;
    style.visuals.selection.bg_fill = ACCENT;
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(24.0, FontFamily::Proportional),
    );
    ctx.set_global_style(style);
}

fn nav_button(ui: &mut egui::Ui, page: &mut Page, target: Page, label: &str) {
    let selected = *page == target;
    if ui
        .add_sized(
            [162.0, 38.0],
            egui::Button::new(
                RichText::new(label)
                    .color(if selected { ACCENT } else { INK })
                    .strong(),
            )
            .fill(if selected {
                ACCENT_SOFT
            } else {
                Color32::TRANSPARENT
            })
            .stroke(Stroke::NONE)
            .corner_radius(8),
        )
        .clicked()
    {
        *page = target;
    }
}

fn preset_row<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    title: &str,
    detail: &str,
    value: &mut T,
    choices: [T; 3],
    label: fn(T) -> &'static str,
) {
    Frame::new()
        .fill(Color32::from_rgb(248, 247, 243))
        .corner_radius(9)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).strong());
                    ui.label(RichText::new(detail).size(11.0).color(MUTED));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    egui::ComboBox::from_id_salt(title)
                        .selected_text(label(*value))
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            for choice in choices {
                                ui.selectable_value(value, choice, label(choice));
                            }
                        });
                });
            });
        });
    ui.add_space(6.0);
}

fn metric_card(ui: &mut egui::Ui, title: &str, value: &str, progress: f32) {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(12)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_min_width(205.0);
            ui.label(RichText::new(title).size(10.0).strong().color(MUTED));
            ui.label(RichText::new(value).size(22.0).strong().color(INK));
            ui.add(egui::ProgressBar::new(progress.clamp(0.0, 1.0)).fill(ACCENT));
        });
}

fn path_field(ui: &mut egui::Ui, title: &str, detail: &str, value: &mut String, valid: bool) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).strong());
            ui.label(RichText::new(detail).size(11.0).color(MUTED));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(if valid { "● Ready" } else { "● Not found" })
                    .size(11.0)
                    .strong()
                    .color(if valid {
                        ACCENT
                    } else {
                        Color32::from_rgb(184, 72, 65)
                    }),
            );
        });
    });
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .hint_text("Choose a folder…"),
    );
}

fn health_row(ui: &mut egui::Ui, name: &str, healthy: bool, detail: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if healthy { "●" } else { "●" }).color(if healthy {
                ACCENT
            } else {
                Color32::from_rgb(191, 119, 49)
            }),
        );
        ui.vertical(|ui| {
            ui.label(RichText::new(name).strong());
            ui.label(RichText::new(detail).size(11.0).color(MUTED));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(if healthy { "Ready" } else { "Needs attention" }).color(
                    if healthy {
                        ACCENT
                    } else {
                        Color32::from_rgb(191, 119, 49)
                    },
                ),
            );
        });
    });
    ui.separator();
}

fn gib(bytes: u64) -> f32 {
    bytes as f32 / 1024.0 / 1024.0 / 1024.0
}

fn relative_time(at: SystemTime) -> &'static str {
    match SystemTime::now()
        .duration_since(at)
        .unwrap_or_default()
        .as_secs()
    {
        0..=4 => "now",
        5..=59 => "seconds ago",
        _ => "earlier",
    }
}
