use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, RichText, Stroke};
use tokio::runtime::Runtime;

use crate::{
    backfill::{BackfillController, BackfillStatus},
    config::EditableSettings,
    control::RuntimeControl,
    db::Database,
    graph_view::{FieldTelemetry, GraphColorMode, GraphViewport},
    models::{DashboardSnapshot, FrontierEntry, PageRecord},
    settings::{SettingsManager, SettingsSnapshot},
};

#[derive(Clone)]
pub struct UiContext {
    pub runtime: Arc<Runtime>,
    pub database: Database,
    pub settings: SettingsManager,
    pub control: RuntimeControl,
    pub backfill: BackfillController,
}

pub struct HungerApp {
    context: UiContext,
    graph: GraphViewport,
    dashboard: DashboardSnapshot,
    settings_snapshot: SettingsSnapshot,
    editable_settings: EditableSettings,
    settings_dirty: bool,
    graph_mode: GraphColorMode,
    backfill_status: BackfillStatus,
    last_refresh: Instant,
    status_line: String,
    quick_seed: String,
}

impl HungerApp {
    pub fn new(cc: &eframe::CreationContext<'_>, context: UiContext) -> Self {
        apply_theme(&cc.egui_ctx);
        let settings_snapshot = context.runtime.block_on(context.settings.snapshot());
        let dashboard = context
            .runtime
            .block_on(context.database.snapshot())
            .unwrap_or_else(|_| empty_snapshot());

        Self {
            editable_settings: settings_snapshot.settings.clone(),
            settings_dirty: false,
            graph_mode: GraphColorMode::Novelty,
            backfill_status: context.runtime.block_on(context.backfill.snapshot()),
            settings_snapshot,
            dashboard,
            context,
            graph: GraphViewport::default(),
            last_refresh: Instant::now(),
            status_line: "Crawler online.".to_string(),
            quick_seed: String::new(),
        }
    }

    fn refresh(&mut self) {
        match self
            .context
            .runtime
            .block_on(self.context.database.snapshot())
        {
            Ok(snapshot) => {
                self.dashboard = snapshot;
            }
            Err(error) => {
                self.status_line = format!("Snapshot failed: {error}");
            }
        }

        self.settings_snapshot = self
            .context
            .runtime
            .block_on(self.context.settings.snapshot());
        self.backfill_status = self
            .context
            .runtime
            .block_on(self.context.backfill.snapshot());
    }

    fn save_settings(&mut self) {
        match self
            .context
            .runtime
            .block_on(self.context.settings.update(self.editable_settings.clone()))
        {
            Ok(snapshot) => {
                self.settings_snapshot = snapshot;
                self.editable_settings = self.settings_snapshot.settings.clone();
                self.settings_dirty = false;
                self.status_line = if self.settings_snapshot.pending_restart.is_empty() {
                    "Settings saved. Runtime changes are active.".to_string()
                } else {
                    format!(
                        "Settings saved. Restart required for: {}",
                        self.settings_snapshot.pending_restart.join(", ")
                    )
                };
                let seeds = self.settings_snapshot.settings.seed_urls.clone();
                if let Err(error) = self
                    .context
                    .runtime
                    .block_on(self.context.database.enqueue_seed_urls(&seeds))
                {
                    self.status_line =
                        format!("Settings saved, but enqueuing seeds failed: {error}");
                }
                self.refresh();
            }
            Err(error) => {
                self.status_line = format!("Saving settings failed: {error}");
            }
        }
    }

    fn enqueue_seed(&mut self, url: String) {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return;
        }

        match self
            .context
            .runtime
            .block_on(self.context.database.enqueue_link(
                trimmed,
                None,
                Some("operator seed"),
                0,
                0.5,
            )) {
            Ok(()) => {
                self.quick_seed.clear();
                self.status_line = format!("Queued seed: {trimmed}");
                self.refresh();
            }
            Err(error) => {
                self.status_line = format!("Queueing seed failed: {error}");
            }
        }
    }

    fn start_backfill(&mut self) {
        let started = self.context.backfill.start(
            &self.context.runtime,
            self.context.database.clone(),
            self.context.settings.clone(),
        );
        if started {
            self.status_line = "Started semantic backfill.".to_string();
            self.backfill_status.running = true;
        } else {
            self.status_line = "Semantic backfill is already running.".to_string();
        }
    }

    fn draw_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar")
            .exact_height(54.0)
            .frame(egui::Frame::default().fill(Color32::from_rgb(9, 12, 20)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(
                        RichText::new("HUNGER//PROTO-01").color(Color32::from_rgb(235, 240, 245)),
                    );
                    ui.label(
                        RichText::new("novelty-seeking crawler")
                            .monospace()
                            .color(Color32::from_rgb(116, 142, 162)),
                    );
                    ui.add_space(18.0);

                    let paused = self.context.control.is_paused();
                    let button = egui::Button::new(if paused { "RESUME" } else { "PAUSE" }).fill(
                        if paused {
                            Color32::from_rgb(84, 214, 255)
                        } else {
                            Color32::from_rgb(255, 86, 86)
                        },
                    );
                    if ui.add(button).clicked() {
                        self.context.control.toggle();
                    }

                    if ui.button("REFRESH").clicked() {
                        self.refresh();
                    }
                    let backfill_button = egui::Button::new(if self.backfill_status.running {
                        "BACKFILL RUNNING"
                    } else {
                        "BACKFILL SEMANTICS"
                    })
                    .fill(if self.backfill_status.running {
                        Color32::from_rgb(255, 191, 102)
                    } else {
                        Color32::from_rgb(28, 42, 62)
                    });
                    if ui
                        .add_enabled(!self.backfill_status.running, backfill_button)
                        .clicked()
                    {
                        self.start_backfill();
                    }

                    ui.separator();
                    ui.label(
                        RichText::new("FIELD MODE")
                            .monospace()
                            .size(11.0)
                            .color(Color32::from_rgb(109, 133, 153)),
                    );
                    for mode in GraphColorMode::ALL {
                        let selected = self.graph_mode == mode;
                        let response = ui.selectable_label(selected, mode.label());
                        if response.clicked() {
                            self.graph_mode = mode;
                        }
                    }

                    ui.separator();
                    ui.label(
                        RichText::new(if paused {
                            "crawler paused"
                        } else {
                            "crawler live"
                        })
                        .color(if paused {
                            Color32::from_rgb(255, 191, 102)
                        } else {
                            Color32::from_rgb(126, 232, 166)
                        }),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(&self.status_line).color(Color32::from_rgb(139, 164, 183)),
                    );
                });
            });
    }

    fn draw_side_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("side_panel")
            .resizable(true)
            .default_width(356.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.draw_metrics(ui);
                    ui.add_space(12.0);
                    self.draw_seed_box(ui);
                    ui.add_space(18.0);
                    self.draw_settings(ui);
                });
            });
    }

    fn draw_metrics(&self, ui: &mut egui::Ui) {
        let panel = panel_frame(Color32::from_rgb(13, 18, 28));
        panel.show(ui, |ui| {
            ui.label(section_title("Status"));
            ui.add_space(6.0);
            draw_status_bar(
                ui,
                self.dashboard.total_pages,
                self.dashboard.queued_pages,
                self.dashboard.failed_pages,
            );
            ui.add_space(10.0);
            draw_backfill_status(ui, &self.backfill_status);
        });
    }

    fn draw_seed_box(&mut self, ui: &mut egui::Ui) {
        panel_frame(Color32::from_rgb(11, 16, 24)).show(ui, |ui| {
            ui.label(section_title("Inject Seed"));
            ui.add(
                egui::TextEdit::singleline(&mut self.quick_seed)
                    .hint_text("https://example.com")
                    .desired_width(f32::INFINITY),
            );
            if ui.button("QUEUE SEED").clicked() {
                self.enqueue_seed(self.quick_seed.clone());
            }
        });
    }

    fn draw_settings(&mut self, ui: &mut egui::Ui) {
        panel_frame(Color32::from_rgb(10, 15, 23)).show(ui, |ui| {
            ui.label(section_title("Settings"));
            ui.label(
                RichText::new(format!("saved to {}", self.settings_snapshot.settings_path))
                    .monospace()
                    .size(11.0)
                    .color(Color32::from_rgb(109, 133, 153)),
            );

            ui.add_space(10.0);
            form_text(ui, "Database URL", &mut self.editable_settings.database_url);
            form_text(ui, "LLM Base URL", &mut self.editable_settings.llm_base_url);
            form_text(ui, "Chat Model", &mut self.editable_settings.llm_model);

            ui.label(field_label("API Key"));
            ui.add(
                egui::TextEdit::singleline(&mut self.editable_settings.llm_api_key)
                    .password(true)
                    .desired_width(f32::INFINITY),
            );

            let embedding = self
                .editable_settings
                .embedding_model
                .get_or_insert_with(String::new);
            form_text(ui, "Embedding Model", embedding);
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(
                    &mut self.editable_settings.page_semantic_map_enabled,
                    "Page semantic embeddings",
                );
                ui.checkbox(
                    &mut self.editable_settings.llm_semantic_map_enabled,
                    "LLM-response embeddings",
                );
            });
            ui.label(
                RichText::new("semantic modes use the embedding model and can be disabled to keep crawl latency down")
                    .size(11.0)
                    .color(Color32::from_rgb(109, 133, 153)),
            );

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                numeric_drag(
                    ui,
                    "Timeout (s)",
                    &mut self.editable_settings.request_timeout_secs,
                    1,
                    120,
                );
                numeric_drag(
                    ui,
                    "Idle (ms)",
                    &mut self.editable_settings.crawl_idle_ms,
                    100,
                    10_000,
                );
            });
            ui.horizontal(|ui| {
                numeric_drag(
                    ui,
                    "LLM Timeout (s)",
                    &mut self.editable_settings.llm_timeout_secs,
                    5,
                    600,
                );
                ui.add_space(8.0);
            });
            ui.horizontal(|ui| {
                numeric_drag_usize(
                    ui,
                    "Max Links",
                    &mut self.editable_settings.max_links_per_page,
                    1,
                    24,
                );
                threshold_drag(ui, "Expand", &mut self.editable_settings.expand_threshold);
            });
            threshold_drag(
                ui,
                "Sample Threshold",
                &mut self.editable_settings.sample_threshold,
            );

            ui.add_space(8.0);
            ui.label(field_label("Seed URLs"));
            let mut seed_text = self.editable_settings.seed_urls.join("\n");
            if ui
                .add(
                    egui::TextEdit::multiline(&mut seed_text)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                self.editable_settings.seed_urls = seed_text
                    .lines()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("SAVE SETTINGS").clicked() {
                    self.save_settings();
                }
                if ui.button("RELOAD").clicked() {
                    self.refresh();
                    self.editable_settings = self.settings_snapshot.settings.clone();
                    self.settings_dirty = false;
                    self.status_line = "Reloaded saved settings.".to_string();
                }
            });

            self.settings_dirty = self.editable_settings != self.settings_snapshot.settings;

            if self.settings_dirty {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("unsaved changes")
                        .monospace()
                        .size(11.0)
                        .color(Color32::from_rgb(255, 201, 92)),
                );
            }

            if !self.settings_snapshot.pending_restart.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "restart required for: {}",
                        self.settings_snapshot.pending_restart.join(", ")
                    ))
                    .color(Color32::from_rgb(255, 184, 96)),
                );
            }
        });
    }

    fn draw_center(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(Color32::from_rgb(4, 7, 13)))
            .show(ctx, |ui| {
                let graph_height = (ui.available_height() * 0.57).max(320.0);
                panel_frame(Color32::from_rgb(6, 9, 16)).show(ui, |ui| {
                    ui.set_height(graph_height);
                    self.graph.draw(ui, &self.dashboard, self.graph_mode);
                });
                let telemetry = self.graph.telemetry();

                ui.add_space(12.0);
                ui.columns(3, |columns| {
                    self.draw_command_panel(&mut columns[0], &telemetry);
                    self.draw_pages_list(&mut columns[1]);
                    self.draw_frontier_list(&mut columns[2]);
                });
            });
    }

    fn draw_command_panel(&self, ui: &mut egui::Ui, telemetry: &FieldTelemetry) {
        panel_frame(Color32::from_rgb(9, 13, 20)).show(ui, |ui| {
            ui.label(section_title("Command Radar"));
            draw_command_radar(ui, telemetry, &self.backfill_status);
        });
    }

    fn draw_pages_list(&self, ui: &mut egui::Ui) {
        panel_frame(Color32::from_rgb(9, 13, 20)).show(ui, |ui| {
            ui.label(section_title("Recent Pages"));
            egui::ScrollArea::vertical()
                .max_height(340.0)
                .show(ui, |ui| {
                    for page in self.dashboard.pages.iter().take(24) {
                        draw_page_card(ui, page);
                        ui.add_space(8.0);
                    }
                });
        });
    }

    fn draw_frontier_list(&self, ui: &mut egui::Ui) {
        panel_frame(Color32::from_rgb(9, 13, 20)).show(ui, |ui| {
            ui.label(section_title("Frontier"));
            egui::ScrollArea::vertical()
                .max_height(340.0)
                .show(ui, |ui| {
                    for item in self.dashboard.frontier.iter().take(24) {
                        draw_frontier_card(ui, item);
                        ui.add_space(8.0);
                    }
                });
        });
    }
}

impl eframe::App for HungerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_refresh.elapsed() >= Duration::from_millis(900) {
            self.refresh();
            self.last_refresh = Instant::now();
        }

        self.draw_top_bar(ctx);
        self.draw_side_panel(ctx);
        self.draw_center(ctx);

        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(5, 8, 14);
    visuals.window_fill = Color32::from_rgb(5, 8, 14);
    visuals.extreme_bg_color = Color32::from_rgb(6, 10, 16);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(14, 20, 31);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(20, 29, 44);
    visuals.widgets.active.bg_fill = Color32::from_rgb(28, 42, 62);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(37, 50, 69));
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(41, 54, 74));
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(84, 214, 255));
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(255, 96, 96));
    visuals.selection.bg_fill = Color32::from_rgb(255, 70, 70);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(255, 190, 190));
    visuals.override_text_color = Some(Color32::from_rgb(224, 232, 242));
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 9.0);
    ctx.set_style(style);
}

fn panel_frame(fill: Color32) -> egui::Frame {
    egui::Frame::default()
        .fill(fill)
        .stroke(Stroke::new(1.0, Color32::from_rgb(26, 34, 48)))
        .inner_margin(12.0)
}

fn section_title(title: &str) -> RichText {
    RichText::new(title)
        .size(20.0)
        .color(Color32::from_rgb(233, 238, 245))
}

fn field_label(label: &str) -> RichText {
    RichText::new(label)
        .monospace()
        .size(11.0)
        .color(Color32::from_rgb(109, 133, 153))
}

fn form_text(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(field_label(label));
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
}

fn numeric_drag(ui: &mut egui::Ui, label: &str, value: &mut u64, min: u64, max: u64) {
    ui.vertical(|ui| {
        ui.label(field_label(label));
        ui.add(egui::DragValue::new(value).range(min..=max));
    });
}

fn numeric_drag_usize(ui: &mut egui::Ui, label: &str, value: &mut usize, min: usize, max: usize) {
    ui.vertical(|ui| {
        ui.label(field_label(label));
        ui.add(egui::DragValue::new(value).range(min..=max));
    });
}

fn threshold_drag(ui: &mut egui::Ui, label: &str, value: &mut f32) {
    ui.vertical(|ui| {
        ui.label(field_label(label));
        ui.add(egui::Slider::new(value, 0.0..=1.0).show_value(true));
    });
}

fn draw_status_bar(ui: &mut egui::Ui, done: i64, queued: i64, failed: i64) {
    let desired = egui::vec2(ui.available_width(), 86.0);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let total = (done + queued + failed).max(0) as f32;
    let bar_rect = egui::Rect::from_min_size(
        rect.min + egui::vec2(0.0, 28.0),
        egui::vec2(rect.width(), 28.0),
    );

    painter.rect_filled(bar_rect, 6.0, Color32::from_rgb(15, 20, 30));
    painter.rect_stroke(
        bar_rect,
        6.0,
        Stroke::new(1.0, Color32::from_rgb(35, 46, 62)),
        egui::StrokeKind::Inside,
    );

    let segments = [
        ("DONE", done.max(0) as f32, Color32::from_rgb(114, 229, 149)),
        (
            "QUEUED",
            queued.max(0) as f32,
            Color32::from_rgb(255, 201, 92),
        ),
        (
            "FAILED",
            failed.max(0) as f32,
            Color32::from_rgb(255, 92, 92),
        ),
    ];

    if total <= f32::EPSILON {
        painter.text(
            bar_rect.center(),
            egui::Align2::CENTER_CENTER,
            "No crawl data yet",
            egui::FontId::monospace(11.0),
            Color32::from_rgb(101, 122, 143),
        );
        return;
    }

    let mut left = bar_rect.left();
    for (label, value, color) in segments {
        if value <= 0.0 {
            continue;
        }

        let width = if (left - bar_rect.left()).abs() < f32::EPSILON {
            (bar_rect.width() * (value / total)).round()
        } else {
            (bar_rect.right() - left).min((bar_rect.width() * (value / total)).round())
        };
        let segment_rect = egui::Rect::from_min_max(
            egui::pos2(left, bar_rect.top()),
            egui::pos2((left + width).min(bar_rect.right()), bar_rect.bottom()),
        );
        if segment_rect.width() <= 0.0 {
            continue;
        }

        painter.rect_filled(segment_rect, 0.0, color);
        let center_x = segment_rect.center().x;
        painter.text(
            egui::pos2(center_x, rect.top() + 2.0),
            egui::Align2::CENTER_TOP,
            format!("{}", value as i64),
            egui::FontId::proportional(24.0),
            Color32::from_rgb(238, 243, 249),
        );
        painter.text(
            egui::pos2(center_x, bar_rect.bottom() + 8.0),
            egui::Align2::CENTER_TOP,
            label,
            egui::FontId::monospace(10.5),
            Color32::from_rgb(114, 135, 154),
        );
        left = segment_rect.right();
    }
}

fn draw_backfill_status(ui: &mut egui::Ui, status: &BackfillStatus) {
    let tone = if status.running {
        Color32::from_rgb(255, 191, 102)
    } else if status.last_error.is_some() {
        Color32::from_rgb(255, 96, 96)
    } else {
        Color32::from_rgb(114, 229, 149)
    };

    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("semantic backfill")
                .monospace()
                .size(11.0)
                .color(Color32::from_rgb(109, 133, 153)),
        );
        ui.label(
            RichText::new(if status.running { "running" } else { "idle" })
                .monospace()
                .size(11.5)
                .color(tone),
        );
        ui.label(
            RichText::new(format!("{}/{}", status.processed, status.total))
                .monospace()
                .size(11.5)
                .color(Color32::from_rgb(208, 219, 231)),
        );
    });

    if status.total > 0 {
        let progress = (status.processed as f32 / status.total as f32).clamp(0.0, 1.0);
        ui.add(
            egui::ProgressBar::new(progress)
                .fill(Color32::from_rgb(255, 96, 96))
                .desired_width(f32::INFINITY)
                .text(format!("{} // {}", status.phase, progress_label(status))),
        );
    } else {
        ui.label(
            RichText::new(&status.phase)
                .size(11.0)
                .color(Color32::from_rgb(139, 164, 183)),
        );
    }

    if let Some(url) = &status.current_url {
        ui.label(
            RichText::new(url)
                .size(10.5)
                .color(Color32::from_rgb(113, 204, 255)),
        );
    }

    if let Some(error) = &status.last_error {
        ui.label(
            RichText::new(error)
                .size(10.5)
                .color(Color32::from_rgb(255, 116, 116)),
        );
    }
}

fn progress_label(status: &BackfillStatus) -> String {
    format!(
        "page {}  llm {}",
        status.page_processed, status.llm_processed
    )
}

fn draw_command_radar(ui: &mut egui::Ui, telemetry: &FieldTelemetry, status: &BackfillStatus) {
    let desired = egui::vec2(ui.available_width(), 340.0);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 10.0, Color32::from_rgb(7, 10, 18));
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, Color32::from_rgb(36, 46, 62)),
        egui::StrokeKind::Inside,
    );

    let center = rect.center_top() + egui::vec2(0.0, 132.0);
    let radius = rect.width().min(220.0) * 0.36;
    let ring_color = match telemetry.mode {
        GraphColorMode::Novelty => Color32::from_rgba_unmultiplied(255, 96, 96, 36),
        GraphColorMode::PageSemantic => Color32::from_rgba_unmultiplied(92, 225, 255, 42),
        GraphColorMode::JudgmentSemantic => Color32::from_rgba_unmultiplied(255, 176, 72, 44),
    };

    for factor in [0.25, 0.45, 0.68, 0.92] {
        painter.circle_stroke(center, radius * factor, Stroke::new(1.0, ring_color));
    }
    painter.line_segment(
        [
            egui::pos2(center.x - radius, center.y),
            egui::pos2(center.x + radius, center.y),
        ],
        Stroke::new(1.0, ring_color),
    );
    painter.line_segment(
        [
            egui::pos2(center.x, center.y - radius),
            egui::pos2(center.x, center.y + radius),
        ],
        Stroke::new(1.0, ring_color),
    );

    let cluster_count = telemetry.cluster_sizes.len().max(1);
    for (index, cluster) in telemetry.cluster_sizes.iter().take(6).enumerate() {
        let angle = index as f32 / cluster_count as f32 * std::f32::consts::TAU
            - std::f32::consts::FRAC_PI_2;
        let length =
            radius * (cluster.count as f32 / telemetry.total_nodes.max(1) as f32).clamp(0.15, 1.0);
        let end = egui::pos2(
            center.x + angle.cos() * length,
            center.y + angle.sin() * length,
        );
        let color = if telemetry.mode == GraphColorMode::JudgmentSemantic {
            Color32::from_rgb(255, 176, 72)
        } else {
            Color32::from_rgb(84, 214, 255)
        };
        painter.line_segment([center, end], Stroke::new(3.0, color));
        painter.circle_filled(end, 5.0, color);
        painter.text(
            end + egui::vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("C{:02} {}", cluster.cluster_index + 1, cluster.count),
            egui::FontId::monospace(10.5),
            Color32::from_rgb(223, 235, 247),
        );
    }

    painter.text(
        rect.left_top() + egui::vec2(12.0, 12.0),
        egui::Align2::LEFT_TOP,
        "MAGI TELEMETRY",
        egui::FontId::proportional(18.0),
        Color32::from_rgb(239, 243, 247),
    );
    painter.text(
        rect.left_top() + egui::vec2(12.0, 34.0),
        egui::Align2::LEFT_TOP,
        format!(
            "mode={}  coverage={}/{}  resonance={}",
            telemetry.mode.label(),
            telemetry.coverage,
            telemetry.total_nodes,
            telemetry.resonance_links
        ),
        egui::FontId::monospace(11.0),
        Color32::from_rgb(255, 191, 102),
    );

    let footer_top = rect.bottom() - 92.0;
    painter.line_segment(
        [
            egui::pos2(rect.left() + 12.0, footer_top),
            egui::pos2(rect.right() - 12.0, footer_top),
        ],
        Stroke::new(1.0, Color32::from_rgb(34, 44, 60)),
    );
    painter.text(
        egui::pos2(rect.left() + 12.0, footer_top + 12.0),
        egui::Align2::LEFT_TOP,
        format!(
            "avg energy {:.2}  max energy {:.2}",
            telemetry.average_energy, telemetry.max_energy
        ),
        egui::FontId::monospace(11.0),
        Color32::from_rgb(210, 219, 228),
    );

    let mut y = footer_top + 32.0;
    for domain in telemetry.top_domains.iter().take(3) {
        painter.text(
            egui::pos2(rect.left() + 12.0, y),
            egui::Align2::LEFT_TOP,
            format!("{:>2}  {}", domain.count, domain.domain),
            egui::FontId::monospace(10.5),
            Color32::from_rgb(113, 204, 255),
        );
        y += 16.0;
    }

    let status_text = if status.running {
        format!(
            "BACKFILL {} / {}  {}",
            status.processed, status.total, status.phase
        )
    } else if let Some(error) = &status.last_error {
        format!("BACKFILL ERR  {}", truncate(error, 38))
    } else {
        "BACKFILL IDLE".to_string()
    };
    painter.text(
        egui::pos2(rect.right() - 12.0, footer_top + 12.0),
        egui::Align2::RIGHT_TOP,
        status_text,
        egui::FontId::monospace(11.0),
        if status.running {
            Color32::from_rgb(255, 191, 102)
        } else if status.last_error.is_some() {
            Color32::from_rgb(255, 96, 96)
        } else {
            Color32::from_rgb(114, 229, 149)
        },
    );
}

fn truncate(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        value.to_string()
    } else {
        format!("{}...", value.chars().take(max_chars).collect::<String>())
    }
}

fn draw_page_card(ui: &mut egui::Ui, page: &PageRecord) {
    panel_frame(Color32::from_rgb(12, 18, 27)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(&page.title)
                        .strong()
                        .color(Color32::from_rgb(230, 235, 243)),
                );
                ui.hyperlink_to(
                    RichText::new(&page.url)
                        .size(11.0)
                        .color(Color32::from_rgb(113, 204, 255)),
                    &page.url,
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                ui.label(
                    RichText::new(format!("{:.2}", page.food_energy))
                        .monospace()
                        .color(Color32::from_rgb(255, 97, 97)),
                );
            });
        });
        ui.label(
            RichText::new(&page.reason)
                .size(12.0)
                .color(Color32::from_rgb(136, 158, 176)),
        );
        ui.label(
            RichText::new(&page.summary)
                .size(13.0)
                .color(Color32::from_rgb(210, 219, 228)),
        );

        if let Some(llm) = &page.llm_novelty {
            ui.add_space(8.0);
            egui::CollapsingHeader::new("LLM Response")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "action={} factual={:.2} entity={:.2} relational={:.2} temporal={:.2} expected={:.2}",
                            llm.recommended_action,
                            llm.factual_novelty,
                            llm.entity_novelty,
                            llm.relational_novelty,
                            llm.temporal_novelty,
                            llm.expected_crawl_value
                        ))
                        .monospace()
                        .size(11.0)
                        .color(Color32::from_rgb(118, 203, 255)),
                    );
                    ui.label(
                        RichText::new(&llm.concise_reason)
                            .size(12.5)
                            .color(Color32::from_rgb(223, 229, 238)),
                    );

                    if !llm.analysis.trim().is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(&llm.analysis)
                                .size(12.0)
                                .color(Color32::from_rgb(170, 183, 198)),
                        );
                    }

                    if !llm.likely_novel_aspects.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("likely novel")
                                .monospace()
                                .size(11.0)
                                .color(Color32::from_rgb(255, 94, 94)),
                        );
                        for aspect in &llm.likely_novel_aspects {
                            ui.label(
                                RichText::new(format!("+ {aspect}"))
                                    .size(12.0)
                                    .color(Color32::from_rgb(211, 219, 228)),
                            );
                        }
                    }

                    if !llm.likely_redundant_aspects.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("likely redundant")
                                .monospace()
                                .size(11.0)
                                .color(Color32::from_rgb(255, 191, 102)),
                        );
                        for aspect in &llm.likely_redundant_aspects {
                            ui.label(
                                RichText::new(format!("- {aspect}"))
                                    .size(12.0)
                                    .color(Color32::from_rgb(190, 202, 214)),
                            );
                        }
                    }

                    if !llm.recommended_link_hints.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("link hints")
                                .monospace()
                                .size(11.0)
                                .color(Color32::from_rgb(110, 216, 177)),
                        );
                        for hint in llm.recommended_link_hints.iter().take(4) {
                            ui.label(
                                RichText::new(format!(
                                    "{:.2}  {}",
                                    hint.priority, hint.reason
                                ))
                                .size(12.0)
                                .color(Color32::from_rgb(205, 215, 226)),
                            );
                            ui.hyperlink_to(
                                RichText::new(&hint.link_url)
                                    .size(11.0)
                                    .color(Color32::from_rgb(113, 204, 255)),
                                &hint.link_url,
                            );
                        }
                    }
                });
        }
    });
}

fn draw_frontier_card(ui: &mut egui::Ui, item: &FrontierEntry) {
    panel_frame(Color32::from_rgb(12, 18, 27)).show(ui, |ui| {
        ui.label(
            RichText::new(&item.url)
                .strong()
                .color(Color32::from_rgb(225, 232, 241)),
        );
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("depth {}", item.depth))
                    .monospace()
                    .size(11.0)
                    .color(Color32::from_rgb(112, 137, 157)),
            );
            ui.label(
                RichText::new(format!("energy {:.2}", item.energy))
                    .monospace()
                    .size(11.0)
                    .color(Color32::from_rgb(255, 96, 96)),
            );
            ui.label(RichText::new(&item.status).monospace().size(11.0).color(
                match item.status.as_str() {
                    "processing" => Color32::from_rgb(255, 190, 110),
                    "failed" => Color32::from_rgb(255, 96, 96),
                    _ => Color32::from_rgb(122, 230, 170),
                },
            ));
        });
    });
}

fn empty_snapshot() -> DashboardSnapshot {
    DashboardSnapshot {
        total_pages: 0,
        queued_pages: 0,
        failed_pages: 0,
        pages: Vec::new(),
        frontier: Vec::new(),
        graph_nodes: Vec::new(),
        graph_edges: Vec::new(),
    }
}
