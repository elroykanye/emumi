use crate::{
    android::{AndroidTools, validate_profile_name},
    config::AppConfig,
    model::{AndroidProfile, HostStats, LogEntry, LogLevel, ProfileOptions, SpeedPreset},
    monitor::HostMonitor,
};
use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Frame, Layout, Margin, RichText, Stroke, Vec2,
};
use egui_phosphor::regular as icon;
use std::time::{Duration, Instant, SystemTime};

const CANVAS: Color32 = Color32::from_rgb(246, 247, 245);
const SURFACE: Color32 = Color32::from_rgb(255, 255, 253);
const SIDEBAR: Color32 = Color32::from_rgb(239, 243, 239);
const TEXT: Color32 = Color32::from_rgb(27, 35, 30);
const SOFT: Color32 = Color32::from_rgb(91, 104, 96);
const FAINT: Color32 = Color32::from_rgb(128, 140, 132);
const GREEN: Color32 = Color32::from_rgb(42, 126, 75);
const GREEN_DARK: Color32 = Color32::from_rgb(30, 101, 58);
const GREEN_SOFT: Color32 = Color32::from_rgb(225, 241, 230);
const BORDER: Color32 = Color32::from_rgb(216, 224, 218);
const FIELD: Color32 = Color32::from_rgb(247, 249, 247);
const AMBER: Color32 = Color32::from_rgb(174, 104, 31);
const RED: Color32 = Color32::from_rgb(177, 62, 58);
const BLUE: Color32 = Color32::from_rgb(57, 105, 164);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Androids,
    Monitor,
    Logs,
    Settings,
}

struct CreateDraft {
    name: String,
    device_id: String,
    image_index: usize,
    error: Option<String>,
}

impl Default for CreateDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            device_id: "pixel_7".into(),
            image_index: 0,
            error: None,
        }
    }
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
    show_create: bool,
    create_draft: CreateDraft,
    confirm_delete: Option<String>,
}

impl EmuMiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);
        configure_style(&cc.egui_ctx);

        let mut config = AppConfig::load();
        let tools = AndroidTools::detect(&config.android_sdk_path, &config.jdk_path);
        if config.android_sdk_path.is_empty() {
            config.android_sdk_path = tools
                .sdk_root
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
        }
        if config.jdk_path.is_empty() {
            config.jdk_path = tools
                .java_home
                .as_ref()
                .map(|path| path.display().to_string())
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
            show_create: false,
            create_draft: CreateDraft::default(),
            confirm_delete: None,
        };
        app.log(LogLevel::Success, "EmuMi is ready");
        if app.tools.emulator.is_none() || app.tools.adb.is_none() {
            app.log(LogLevel::Warning, "Android tools need attention");
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
            format!("Found {} profile(s)", self.profiles.len()),
        );
    }

    fn launch_args(options: &ProfileOptions) -> Vec<String> {
        let mut args = vec![
            "-cores".into(),
            options.cores.to_string(),
            "-memory".into(),
            options.memory_mb.to_string(),
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
            .exact_size(76.0)
            .frame(
                Frame::new()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::symmetric(26, 16)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::ANDROID_LOGO).size(30.0).color(GREEN));
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("EmuMi").size(21.0).strong().color(TEXT));
                        ui.label(
                            RichText::new("Android emulators, made easy")
                                .size(12.0)
                                .color(SOFT),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_button(ui, icon::PLUS, "New Android", 142.0).clicked() {
                            self.create_draft = CreateDraft::default();
                            self.show_create = true;
                        }
                    });
                });
            });
    }

    fn sidebar(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("sidebar")
            .exact_size(218.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(SIDEBAR)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::symmetric(15, 22)),
            )
            .show(root, |ui| {
                ui.label(section_label("WORKSPACE"));
                ui.add_space(10.0);
                nav_button(
                    ui,
                    &mut self.page,
                    Page::Androids,
                    icon::DEVICES,
                    "Androids",
                );
                nav_button(ui, &mut self.page, Page::Monitor, icon::GAUGE, "Monitor");
                nav_button(
                    ui,
                    &mut self.page,
                    Page::Logs,
                    icon::TERMINAL_WINDOW,
                    "Logs",
                );
                nav_button(
                    ui,
                    &mut self.page,
                    Page::Settings,
                    icon::GEAR_SIX,
                    "Settings",
                );

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    let ready = self.tools.emulator.is_some() && self.tools.adb.is_some();
                    Frame::new()
                        .fill(SURFACE)
                        .stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(11)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(if ready {
                                        icon::CHECK_CIRCLE
                                    } else {
                                        icon::WARNING_CIRCLE
                                    })
                                    .size(20.0)
                                    .color(if ready {
                                        GREEN
                                    } else {
                                        AMBER
                                    }),
                                );
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(if ready {
                                            "System ready"
                                        } else {
                                            "Setup needed"
                                        })
                                        .size(13.0)
                                        .strong()
                                        .color(TEXT),
                                    );
                                    ui.label(
                                        RichText::new(if ready {
                                            "SDK and ADB found"
                                        } else {
                                            "Check Settings"
                                        })
                                        .size(11.0)
                                        .color(SOFT),
                                    );
                                });
                            });
                        });
                });
            });
    }

    fn androids_page(&mut self, ui: &mut egui::Ui) {
        let refresh = page_header(
            ui,
            "Androids",
            "Launch and manage every Android profile from one place.",
            Some((icon::ARROW_CLOCKWISE, "Refresh")),
        );
        if refresh {
            self.refresh();
        }
        ui.add_space(24.0);

        let rail_width = 310.0_f32.min(ui.available_width() * 0.38);
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(rail_width, ui.available_height()),
                Layout::top_down(Align::Min),
                |ui| self.profile_rail(ui),
            );
            ui.add_space(14.0);
            ui.separator();
            ui.add_space(14.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), ui.available_height()),
                Layout::top_down(Align::Min),
                |ui| match self.selected.filter(|index| *index < self.profiles.len()) {
                    Some(index) => self.profile_detail(ui, index),
                    None => empty_profile(ui),
                },
            );
        });
    }

    fn profile_rail(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(section_label("PROFILES"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                status_pill(ui, &format!("{} total", self.profiles.len()), SOFT, FIELD);
            });
        });
        ui.add_space(10.0);
        if self.profiles.is_empty() {
            empty_card(
                ui,
                icon::DEVICE_MOBILE_SLASH,
                "No profiles found",
                "Create one with the button above.",
            );
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, profile) in self.profiles.iter().enumerate() {
                let selected = self.selected == Some(index);
                let response = Frame::new()
                    .fill(if selected { GREEN_SOFT } else { SURFACE })
                    .stroke(Stroke::new(
                        if selected { 1.5 } else { 1.0 },
                        if selected { GREEN } else { BORDER },
                    ))
                    .corner_radius(12)
                    .inner_margin(Margin::symmetric(14, 13))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(icon::DEVICE_MOBILE)
                                    .size(24.0)
                                    .color(if profile.is_running() { GREEN } else { SOFT }),
                            );
                            ui.add_space(3.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&profile.name).size(14.0).strong().color(TEXT),
                                );
                                ui.label(RichText::new(profile.subtitle()).size(11.5).color(SOFT));
                            });
                        });
                        ui.add_space(8.0);
                        status_pill(
                            ui,
                            if profile.is_running() {
                                "Running"
                            } else {
                                "Stopped"
                            },
                            if profile.is_running() {
                                GREEN_DARK
                            } else {
                                SOFT
                            },
                            if profile.is_running() {
                                GREEN_SOFT
                            } else {
                                FIELD
                            },
                        );
                    })
                    .response
                    .interact(egui::Sense::click());
                if response.clicked() {
                    self.selected = Some(index);
                }
                ui.add_space(9.0);
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
        let mut run_action = None;
        let mut delete = false;

        egui::ScrollArea::vertical().show(ui, |ui| {
            Frame::new()
                .fill(SURFACE)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(14)
                .inner_margin(Margin::symmetric(22, 20))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::ANDROID_LOGO).size(34.0).color(GREEN));
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&profile.name).size(22.0).strong().color(TEXT));
                            ui.label(RichText::new(profile.subtitle()).size(13.0).color(SOFT));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let (run_icon, label) = if profile.is_running() {
                                (icon::STOP, "Stop")
                            } else {
                                (icon::PLAY, "Start")
                            };
                            if primary_button(ui, run_icon, label, 106.0).clicked() {
                                run_action = Some(profile.is_running());
                            }
                            if secondary_button(ui, icon::TRASH, "Delete", 96.0).clicked() {
                                delete = true;
                            }
                        });
                    });
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        status_pill(
                            ui,
                            if profile.is_running() {
                                "Running"
                            } else {
                                "Stopped"
                            },
                            if profile.is_running() {
                                GREEN_DARK
                            } else {
                                SOFT
                            },
                            if profile.is_running() {
                                GREEN_SOFT
                            } else {
                                FIELD
                            },
                        );
                        if let Some(resolution) = &profile.resolution {
                            status_pill(ui, resolution, SOFT, FIELD);
                        }
                    });
                });

            ui.add_space(16.0);
            settings_card(
                ui,
                "Quick setup",
                "Useful defaults without technical noise",
                |ui| {
                    speed_row(ui, options);
                    row_divider(ui);
                    setting_toggle(
                        ui,
                        icon::KEYBOARD,
                        "Use computer keyboard",
                        "Send typing from Linux directly into Android",
                        &mut options.host_keyboard,
                    );
                    row_divider(ui);
                    setting_toggle(
                        ui,
                        icon::FRAME_CORNERS,
                        "Show device frame",
                        "Turn this off for a cleaner, easier-to-resize window",
                        &mut options.device_frame,
                    );
                },
            );

            ui.add_space(14.0);
            Frame::new()
                .fill(SURFACE)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(14)
                .inner_margin(Margin::symmetric(18, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::CollapsingHeader::new(
                        RichText::new(format!("{}  Advanced settings", icon::SLIDERS_HORIZONTAL))
                            .size(14.0)
                            .strong()
                            .color(TEXT),
                    )
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("Change these only when a profile needs special tuning.")
                                .size(12.0)
                                .color(SOFT),
                        );
                        ui.add_space(14.0);
                        advanced_settings(ui, options);
                    });
                });
        });

        if delete {
            self.confirm_delete = Some(profile.name.clone());
        }
        if let Some(was_running) = run_action {
            if was_running {
                let serial = profile.running_serial.as_deref().unwrap_or_default();
                match self.tools.stop(serial) {
                    Ok(()) => self.log(LogLevel::Success, format!("Stopping {}", profile.name)),
                    Err(error) => self.log(LogLevel::Error, error),
                }
            } else {
                let args = Self::launch_args(options);
                match self.tools.configure_input_and_window(
                    &profile.name,
                    options.host_keyboard,
                    options.device_frame,
                ) {
                    Ok(()) => match self.tools.start(&profile, &args) {
                        Ok(()) => self.log(LogLevel::Success, format!("Starting {}", profile.name)),
                        Err(error) => self.log(LogLevel::Error, error),
                    },
                    Err(error) => self.log(LogLevel::Error, error),
                }
            }
        }
    }

    fn monitor_page(&mut self, ui: &mut egui::Ui) {
        page_header(
            ui,
            "Monitor",
            "Host usage and live Android connections.",
            None,
        );
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            metric_card(
                ui,
                icon::CPU,
                "Host CPU",
                &format!("{:.0}%", self.stats.cpu_percent),
                self.stats.cpu_percent / 100.0,
            );
            let memory = if self.stats.memory_total_bytes == 0 {
                0.0
            } else {
                self.stats.memory_used_bytes as f32 / self.stats.memory_total_bytes as f32
            };
            metric_card(
                ui,
                icon::MEMORY,
                "Host memory",
                &format!(
                    "{:.1} / {:.1} GB",
                    gib(self.stats.memory_used_bytes),
                    gib(self.stats.memory_total_bytes)
                ),
                memory,
            );
            let running = self
                .profiles
                .iter()
                .filter(|profile| profile.is_running())
                .count();
            metric_card(
                ui,
                icon::DEVICE_MOBILE,
                "Running",
                &format!("{running} Androids"),
                if self.profiles.is_empty() {
                    0.0
                } else {
                    running as f32 / self.profiles.len() as f32
                },
            );
        });
        ui.add_space(16.0);
        settings_card(ui, "Live Androids", "ADB connections available now", |ui| {
            let mut any = false;
            for profile in self.profiles.iter().filter(|profile| profile.is_running()) {
                any = true;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::CHECK_CIRCLE).size(20.0).color(GREEN));
                    ui.label(RichText::new(&profile.name).strong().color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        status_pill(
                            ui,
                            profile.running_serial.as_deref().unwrap_or("Starting"),
                            GREEN_DARK,
                            GREEN_SOFT,
                        );
                    });
                });
                row_divider(ui);
            }
            if !any {
                empty_state(
                    ui,
                    icon::POWER,
                    "Nothing is running",
                    "Start a profile to see it here.",
                );
            }
        });
    }

    fn logs_page(&mut self, ui: &mut egui::Ui) {
        let clear = page_header(
            ui,
            "Logs",
            "Readable activity from EmuMi and emulator sessions.",
            Some((icon::TRASH, "Clear")),
        );
        if clear {
            self.logs.clear();
        }
        ui.add_space(24.0);
        settings_card(ui, "Recent activity", "Newest events appear first", |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.logs.is_empty() {
                    empty_state(
                        ui,
                        icon::TERMINAL_WINDOW,
                        "No activity yet",
                        "New actions will appear here.",
                    );
                }
                for entry in self.logs.iter().rev() {
                    log_row(ui, entry);
                }
            });
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        let save = page_header(
            ui,
            "Settings",
            "Configure the tools EmuMi uses on this computer.",
            Some((icon::FLOPPY_DISK, "Save")),
        );
        if save {
            self.save_settings();
        }
        ui.add_space(24.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            settings_card(
                ui,
                "Tool locations",
                "Detected automatically, but always editable.",
                |ui| {
                    path_field(
                        ui,
                        icon::ANDROID_LOGO,
                        "Android SDK",
                        "Folder containing emulator/ and platform-tools/",
                        &mut self.config.android_sdk_path,
                        self.tools.emulator.is_some() && self.tools.adb.is_some(),
                    );
                    row_divider(ui);
                    path_field(
                        ui,
                        icon::COFFEE,
                        "Java JDK",
                        "JDK home used by Android command-line tools",
                        &mut self.config.jdk_path,
                        self.tools.java_home.is_some(),
                    );
                    row_divider(ui);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::MAGIC_WAND).size(20.0).color(GREEN));
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Automatic detection").strong().color(TEXT));
                            ui.label(
                                RichText::new("Search common Linux locations again")
                                    .size(11.5)
                                    .color(SOFT),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if secondary_button(ui, icon::MAGNIFYING_GLASS, "Detect", 102.0)
                                .clicked()
                            {
                                self.auto_detect();
                            }
                        });
                    });
                },
            );
            ui.add_space(16.0);
            settings_card(
                ui,
                "System readiness",
                "Everything needed to run accelerated Androids.",
                |ui| {
                    health_row(
                        ui,
                        icon::ANDROID_LOGO,
                        "Android Emulator",
                        self.tools.emulator.is_some(),
                        "Runs virtual devices",
                    );
                    row_divider(ui);
                    health_row(
                        ui,
                        icon::PLUG,
                        "ADB",
                        self.tools.adb.is_some(),
                        "Connects to running Androids",
                    );
                    row_divider(ui);
                    health_row(
                        ui,
                        icon::DEVICE_MOBILE,
                        "AVD Manager",
                        self.tools.avd_manager.is_some(),
                        "Creates and edits profiles",
                    );
                    row_divider(ui);
                    health_row(
                        ui,
                        icon::LIGHTNING,
                        "KVM acceleration",
                        self.tools.kvm_available,
                        "Native-speed virtualization",
                    );
                    row_divider(ui);
                    health_row(
                        ui,
                        icon::COFFEE,
                        "Java",
                        self.tools.java_home.is_some(),
                        "Runs SDK management commands",
                    );
                },
            );
        });
    }

    fn create_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_create {
            return;
        }
        let images = self.tools.installed_system_images();
        let mut open = true;
        let mut create = false;
        let mut cancel = false;
        egui::Window::new("Create a new Android")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(500.0)
            .frame(
                Frame::window(&ctx.global_style())
                    .fill(SURFACE)
                    .inner_margin(22)
                    .corner_radius(14),
            )
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Choose a name and device. EmuMi handles the Android tooling.")
                        .size(12.5)
                        .color(SOFT),
                );
                ui.add_space(18.0);
                form_label(ui, "Name", "Letters, numbers, dashes, and underscores");
                ui.add(
                    egui::TextEdit::singleline(&mut self.create_draft.name)
                        .desired_width(f32::INFINITY)
                        .margin(Vec2::new(11.0, 9.0))
                        .hint_text("e.g. Pixel_7_Work"),
                );
                ui.add_space(14.0);
                form_label(ui, "Device", "The screen and hardware shape");
                egui::ComboBox::from_id_salt("new_device")
                    .selected_text(device_label(&self.create_draft.device_id))
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for (id, label) in device_choices() {
                            ui.selectable_value(
                                &mut self.create_draft.device_id,
                                id.to_owned(),
                                label,
                            );
                        }
                    });
                ui.add_space(14.0);
                form_label(
                    ui,
                    "Android image",
                    "An image already installed in your SDK",
                );
                if images.is_empty() {
                    ui.label(RichText::new("No system image is installed.").color(RED));
                } else {
                    self.create_draft.image_index =
                        self.create_draft.image_index.min(images.len() - 1);
                    egui::ComboBox::from_id_salt("new_image")
                        .selected_text(images[self.create_draft.image_index].label())
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for (index, image) in images.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.create_draft.image_index,
                                    index,
                                    image.label(),
                                );
                            }
                        });
                }
                if let Some(error) = &self.create_draft.error {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(format!("{}  {error}", icon::WARNING_CIRCLE)).color(RED),
                    );
                }
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        create = ui
                            .add_enabled(
                                !images.is_empty() && self.tools.avd_manager.is_some(),
                                egui::Button::new(
                                    RichText::new(format!("{}  Create Android", icon::PLUS))
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(GREEN)
                                .corner_radius(9),
                            )
                            .clicked();
                    });
                });
            });
        self.show_create = open && !cancel;

        if create {
            self.create_draft.error = validate_profile_name(&self.create_draft.name).err();
            if self.create_draft.error.is_none() {
                let name = self.create_draft.name.trim().to_owned();
                let result = self.tools.create_profile(
                    &name,
                    &self.create_draft.device_id,
                    &images[self.create_draft.image_index],
                );
                match result {
                    Ok(()) => {
                        self.log(LogLevel::Success, format!("Created {name}"));
                        self.refresh();
                        self.selected = self
                            .profiles
                            .iter()
                            .position(|profile| profile.name == name);
                        self.show_create = false;
                    }
                    Err(error) => {
                        self.create_draft.error = Some(error.clone());
                        self.log(LogLevel::Error, error);
                    }
                }
            }
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(name) = self.confirm_delete.clone() else {
            return;
        };
        let mut open = true;
        let mut delete = false;
        let mut cancel = false;
        egui::Window::new("Delete Android?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("This permanently removes the “{name}” profile."))
                        .color(TEXT),
                );
                ui.label(RichText::new("The installed system image stays available.").color(SOFT));
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if ui.button("Keep it").clicked() {
                        cancel = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        delete = ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("{}  Delete", icon::TRASH))
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(RED)
                                .corner_radius(9),
                            )
                            .clicked();
                    });
                });
            });
        if delete {
            match self.tools.delete_profile(&name) {
                Ok(()) => {
                    self.log(LogLevel::Success, format!("Deleted {name}"));
                    self.refresh();
                }
                Err(error) => self.log(LogLevel::Error, error),
            }
            self.confirm_delete = None;
        } else if !open || cancel {
            self.confirm_delete = None;
        }
    }

    fn auto_detect(&mut self) {
        let detected = AndroidTools::detect("", "");
        self.config.android_sdk_path = detected
            .sdk_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        self.config.jdk_path = detected
            .java_home
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        self.tools = detected;
        self.log(LogLevel::Info, "Developer tools auto-detected");
    }

    fn save_settings(&mut self) {
        match self.config.save() {
            Ok(path) => {
                self.tools =
                    AndroidTools::detect(&self.config.android_sdk_path, &self.config.jdk_path);
                self.log(
                    LogLevel::Success,
                    format!("Settings saved to {}", path.display()),
                );
            }
            Err(error) => self.log(LogLevel::Error, format!("Could not save: {error}")),
        }
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
                    .fill(CANVAS)
                    .inner_margin(Margin::symmetric(30, 28)),
            )
            .show(ui, |ui| match self.page {
                Page::Androids => self.androids_page(ui),
                Page::Monitor => self.monitor_page(ui),
                Page::Logs => self.logs_page(ui),
                Page::Settings => self.settings_page(ui),
            });
        self.create_dialog(ui.ctx());
        self.delete_dialog(ui.ctx());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.config.save();
    }
}

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 9.0);
    style.spacing.interact_size.y = 38.0;
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = CANVAS;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = FIELD;
    style.visuals.faint_bg_color = FIELD;
    style.visuals.widgets.inactive.fg_stroke.color = TEXT;
    style.visuals.widgets.hovered.bg_fill = GREEN_SOFT;
    style.visuals.widgets.active.bg_fill = GREEN_SOFT;
    style.visuals.selection.bg_fill = GREEN;
    style.text_styles.insert(
        egui::TextStyle::Body,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        FontId::new(13.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(28.0, FontFamily::Proportional),
    );
    ctx.set_global_style(style);
}

fn page_header(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    action: Option<(&str, &str)>,
) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(28.0).strong().color(TEXT));
            ui.label(RichText::new(subtitle).size(13.0).color(SOFT));
        });
        if let Some((action_icon, label)) = action {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                clicked = secondary_button(ui, action_icon, label, 104.0).clicked();
            });
        }
    });
    clicked
}

fn nav_button(ui: &mut egui::Ui, page: &mut Page, target: Page, nav_icon: &str, label: &str) {
    let selected = *page == target;
    if ui
        .add_sized(
            [188.0, 44.0],
            egui::Button::new(
                RichText::new(format!("{nav_icon}   {label}"))
                    .size(14.0)
                    .strong()
                    .color(if selected { GREEN_DARK } else { TEXT }),
            )
            .fill(if selected {
                GREEN_SOFT
            } else {
                Color32::TRANSPARENT
            })
            .stroke(Stroke::NONE)
            .corner_radius(9),
        )
        .clicked()
    {
        *page = target;
    }
    ui.add_space(3.0);
}

fn primary_button(ui: &mut egui::Ui, button_icon: &str, label: &str, width: f32) -> egui::Response {
    ui.add_sized(
        [width, 40.0],
        egui::Button::new(
            RichText::new(format!("{button_icon}  {label}"))
                .strong()
                .color(Color32::WHITE),
        )
        .fill(GREEN)
        .stroke(Stroke::NONE)
        .corner_radius(9),
    )
}

fn secondary_button(
    ui: &mut egui::Ui,
    button_icon: &str,
    label: &str,
    width: f32,
) -> egui::Response {
    ui.add_sized(
        [width, 40.0],
        egui::Button::new(
            RichText::new(format!("{button_icon}  {label}"))
                .strong()
                .color(TEXT),
        )
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(9),
    )
}

fn section_label(text: &str) -> RichText {
    RichText::new(text).size(10.5).strong().color(FAINT)
}

fn status_pill(ui: &mut egui::Ui, text: &str, color: Color32, fill: Color32) {
    Frame::new()
        .fill(fill)
        .corner_radius(20)
        .inner_margin(Margin::symmetric(9, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).strong().color(color));
        });
}

fn settings_card(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(14)
        .inner_margin(Margin::symmetric(20, 18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(16.0).strong().color(TEXT));
            ui.label(RichText::new(subtitle).size(12.0).color(SOFT));
            ui.add_space(18.0);
            contents(ui);
        });
}

fn speed_row(ui: &mut egui::Ui, options: &mut ProfileOptions) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon::GAUGE).size(21.0).color(GREEN));
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.label(RichText::new("Performance").size(13.5).strong().color(TEXT));
            ui.label(
                RichText::new("CPU and memory allocated when Android starts")
                    .size(11.5)
                    .color(SOFT),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for choice in SpeedPreset::ALL.into_iter().rev() {
                let selected = options.speed == choice;
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(choice.label())
                                .size(11.5)
                                .strong()
                                .color(if selected { GREEN_DARK } else { SOFT }),
                        )
                        .fill(if selected { GREEN_SOFT } else { FIELD })
                        .stroke(Stroke::new(1.0, if selected { GREEN } else { BORDER }))
                        .corner_radius(7),
                    )
                    .clicked()
                {
                    options.speed = choice;
                    (options.cores, options.memory_mb) = match choice {
                        SpeedPreset::Efficient => (2, 2048),
                        SpeedPreset::Balanced => (4, 4096),
                        SpeedPreset::Fast => (8, 8192),
                    };
                }
            }
        });
    });
}

fn setting_toggle(ui: &mut egui::Ui, row_icon: &str, title: &str, detail: &str, value: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(row_icon).size(21.0).color(GREEN));
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(13.5).strong().color(TEXT));
            ui.label(RichText::new(detail).size(11.5).color(SOFT));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.checkbox(value, "");
        });
    });
}

fn advanced_settings(ui: &mut egui::Ui, options: &mut ProfileOptions) {
    egui::Grid::new("advanced_grid")
        .num_columns(2)
        .min_col_width(148.0)
        .spacing([24.0, 14.0])
        .show(ui, |ui| {
            advanced_label(ui, "CPU cores", "Virtual processor count");
            ui.add(egui::Slider::new(&mut options.cores, 1..=16));
            ui.end_row();
            advanced_label(ui, "Memory", "Maximum guest RAM");
            ui.add(egui::Slider::new(&mut options.memory_mb, 512..=16384).suffix(" MB"));
            ui.end_row();
            advanced_label(ui, "ADB port", "0 means automatic");
            let mut port = options.adb_port.unwrap_or(0);
            if ui
                .add(egui::DragValue::new(&mut port).range(0..=65535))
                .changed()
            {
                options.adb_port = (port != 0).then_some(port);
            }
            ui.end_row();
            advanced_label(ui, "Graphics", "Rendering backend");
            egui::ComboBox::from_id_salt("gpu_mode")
                .selected_text(&options.gpu_mode)
                .width(160.0)
                .show_ui(ui, |ui| {
                    for mode in ["auto", "host", "swiftshader_indirect"] {
                        ui.selectable_value(&mut options.gpu_mode, mode.to_owned(), mode);
                    }
                });
            ui.end_row();
            advanced_label(ui, "Next boot", "Ignore saved snapshot once");
            ui.checkbox(&mut options.cold_boot, "Cold boot");
            ui.end_row();
        });
}

fn advanced_label(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(title).strong().color(TEXT));
        ui.label(RichText::new(detail).size(11.0).color(SOFT));
    });
}

fn metric_card(ui: &mut egui::Ui, card_icon: &str, title: &str, value: &str, progress: f32) {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(14)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_min_width(205.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(card_icon).size(20.0).color(GREEN));
                ui.label(section_label(&title.to_uppercase()));
            });
            ui.add_space(8.0);
            ui.label(RichText::new(value).size(21.0).strong().color(TEXT));
            ui.add_space(7.0);
            ui.add(
                egui::ProgressBar::new(progress.clamp(0.0, 1.0))
                    .fill(GREEN)
                    .desired_width(185.0),
            );
        });
}

fn path_field(
    ui: &mut egui::Ui,
    field_icon: &str,
    title: &str,
    detail: &str,
    value: &mut String,
    valid: bool,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(field_icon).size(21.0).color(GREEN));
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(13.5).strong().color(TEXT));
            ui.label(RichText::new(detail).size(11.5).color(SOFT));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            status_pill(
                ui,
                if valid { "Ready" } else { "Not found" },
                if valid { GREEN_DARK } else { RED },
                if valid {
                    GREEN_SOFT
                } else {
                    Color32::from_rgb(251, 235, 233)
                },
            );
        });
    });
    ui.add_space(8.0);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .margin(Vec2::new(11.0, 9.0))
            .hint_text("Enter a folder path"),
    );
}

fn health_row(ui: &mut egui::Ui, row_icon: &str, name: &str, healthy: bool, detail: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(row_icon)
                .size(20.0)
                .color(if healthy { GREEN } else { AMBER }),
        );
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(name).size(13.5).strong().color(TEXT));
            ui.label(RichText::new(detail).size(11.5).color(SOFT));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            status_pill(
                ui,
                if healthy { "Ready" } else { "Needs attention" },
                if healthy { GREEN_DARK } else { AMBER },
                if healthy {
                    GREEN_SOFT
                } else {
                    Color32::from_rgb(252, 241, 225)
                },
            );
        });
    });
}

fn log_row(ui: &mut egui::Ui, entry: &LogEntry) {
    let (event_icon, color, label) = match entry.level {
        LogLevel::Info => (icon::INFO, BLUE, "Info"),
        LogLevel::Success => (icon::CHECK_CIRCLE, GREEN, "Success"),
        LogLevel::Warning => (icon::WARNING_CIRCLE, AMBER, "Warning"),
        LogLevel::Error => (icon::X_CIRCLE, RED, "Error"),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(event_icon).size(20.0).color(color));
        ui.vertical(|ui| {
            ui.label(RichText::new(&entry.message).size(13.0).color(TEXT));
            ui.label(RichText::new(label).size(10.5).strong().color(color));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(relative_time(entry.at))
                    .size(11.0)
                    .color(FAINT),
            );
        });
    });
    row_divider(ui);
}

fn empty_profile(ui: &mut egui::Ui) {
    empty_card(
        ui,
        icon::DEVICE_MOBILE,
        "Select an Android",
        "Its controls and settings will appear here.",
    );
}

fn empty_card(ui: &mut egui::Ui, state_icon: &str, title: &str, detail: &str) {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(14)
        .inner_margin(24)
        .show(ui, |ui| {
            empty_state(ui, state_icon, title, detail);
        });
}

fn empty_state(ui: &mut egui::Ui, state_icon: &str, title: &str, detail: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(10.0);
        ui.label(RichText::new(state_icon).size(30.0).color(FAINT));
        ui.label(RichText::new(title).size(14.0).strong().color(TEXT));
        ui.label(RichText::new(detail).size(12.0).color(SOFT));
        ui.add_space(10.0);
    });
}

fn form_label(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.label(RichText::new(title).size(13.0).strong().color(TEXT));
    ui.label(RichText::new(detail).size(11.0).color(SOFT));
    ui.add_space(5.0);
}

fn device_choices() -> [(&'static str, &'static str); 5] {
    [
        ("pixel_7", "Pixel 7 · recommended"),
        ("pixel_9", "Pixel 9"),
        ("medium_phone", "Medium phone"),
        ("pixel_tablet", "Pixel Tablet"),
        ("medium_tablet", "Medium tablet"),
    ]
}

fn device_label(id: &str) -> &str {
    device_choices()
        .into_iter()
        .find(|(device_id, _)| *device_id == id)
        .map(|(_, label)| label)
        .unwrap_or(id)
}

fn row_divider(ui: &mut egui::Ui) {
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(10.0);
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
        0..=4 => "just now",
        5..=59 => "seconds ago",
        _ => "earlier",
    }
}
