use super::*;
use crate::ai::{
    self, AiSettings, CredentialAction, CredentialJob, Provider, Session, Verdict, Worker,
};

#[derive(Default)]
pub(super) struct CoachUi {
    pub session: Session,
    worker: Option<Worker>,
    pub(super) open: bool,
    draft: AiSettings,
    key: String,
    key_entry: String,
    keyring: Option<CredentialJob>,
    keyring_action: u8,
    keyring_attempted: bool,
    keyring_status: String,
    connection: Option<Receiver<Result<String, String>>>,
    connection_status: String,
}

impl InkarchyApp {
    pub(super) fn coach_enabled(&self) -> bool {
        self.note
            .as_ref()
            .is_some_and(|n| self.lib.index.ai.enabled_notebooks.contains(&n.id))
    }

    pub(super) fn open_coach(&mut self) {
        self.coach.open = true;
        self.coach.draft = self.lib.index.ai.clone();
    }

    pub(super) fn tick_coach(&mut self, ctx: &Context) {
        let note_id = self.note.as_ref().map(|n| n.id);
        if note_id != self.coach.session.note {
            self.coach.session.reset(note_id);
        }
        if let Some(done) = self.coach.worker.as_ref().and_then(Worker::poll) {
            self.coach.session.receive(done);
            ctx.request_repaint();
        }
        if let Some(done) = self.coach.keyring.as_ref().and_then(CredentialJob::poll) {
            ctx.request_repaint();
            self.coach.keyring = None;
            match done {
                Ok(value) => {
                    if self.coach.keyring_action == 0 {
                        self.coach.key = value;
                    }
                    self.coach.keyring_status = match self.coach.keyring_action {
                        0 => "API key loaded from system keyring.",
                        1 => "API key saved in system keyring.",
                        _ => "Saved API key removed.",
                    }
                    .into();
                    self.coach.session.invalidate(Instant::now());
                }
                Err(error) => self.coach.keyring_status = error,
            }
        }
        if let Some(result) = self
            .coach
            .connection
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(_) => Some(Err("Connection check stopped.".into())),
            })
        {
            self.coach.connection = None;
            ctx.request_repaint();
            self.coach.connection_status = result.unwrap_or_else(|error| error);
        }
        let uses_api = (self.coach_enabled() && self.lib.index.ai.provider == Provider::Gemini)
            || (self.coach.open && self.coach.draft.provider == Provider::Gemini);
        if uses_api && self.coach.key.is_empty() && !self.coach.keyring_attempted {
            self.coach.keyring_attempted = true;
            self.coach.keyring_action = 0;
            self.coach.keyring = Some(CredentialJob::start(CredentialAction::Load, ctx));
        }
        if !self.coach_enabled() || !matches!(self.scene, Scene::Desk) {
            return;
        }
        if self.lib.index.ai.provider == Provider::Gemini && self.coach.key.is_empty() {
            return;
        }
        let now = Instant::now();
        if self.coach.session.revision > self.coach.session.evaluated
            && self.coach.session.error.is_none()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.sheet_tabs_on || !self.coach.session.ready(now) {
            return;
        }
        let Some(note) = &self.note else { return };
        if self
            .coach
            .session
            .reference
            .is_some_and(|cell| !note.pages.iter().any(|p| (p.col, p.row) == cell))
        {
            self.coach.session.error =
                Some("The statement page was removed. Select another reference page.".into());
            return;
        }
        let page = self
            .coach
            .session
            .page
            .min(note.pages.len().saturating_sub(1));
        let Some(snapshot) = ai::snapshot(note, page) else {
            return;
        };
        let reference = self
            .coach
            .session
            .reference
            .and_then(|cell| note.pages.iter().position(|p| (p.col, p.row) == cell))
            .filter(|p| *p != page)
            .and_then(|p| ai::snapshot(note, p));
        let request = ai::Request {
            ticket: self.coach.session.ticket(),
            note: snapshot,
            reference,
            note_dir: self.lib.note_dir(note.id),
            look: self.look.clone(),
            context: self.coach.session.result.clone(),
            settings: self.lib.index.ai.clone(),
            key: self.coach.key.clone(),
            created: now,
        };
        if self.coach.worker.is_none() {
            match Worker::new(&self.coach.session, ctx) {
                Ok(worker) => self.coach.worker = Some(worker),
                Err(error) => {
                    self.coach.session.error = Some(error);
                    return;
                }
            }
        }
        match self.coach.worker.as_ref().unwrap().send(request) {
            Ok(()) => self.coach.session.sent(now),
            Err(error) => self.coach.session.error = Some(error),
        }
    }

    /// Append-only ink can retain a recent verdict while the next snapshot runs.
    pub(super) fn coach_ink_changed(&mut self, page: usize) {
        let outside_exercise = self
            .coach
            .session
            .result
            .as_ref()
            .and_then(|r| r.region)
            .zip(self.note.as_ref())
            .is_some_and(|([x, y, w, h], note)| {
                let region = Rect::from_min_size(
                    pos2(x * note.page_w, y * note.page_h),
                    vec2(w * note.page_w, h * note.page_h),
                )
                .expand(24.0);
                note.pages
                    .get(page)
                    .and_then(|p| p.strokes.last())
                    .and_then(|s| s.bbox())
                    .is_some_and(|(a, b)| !region.intersects(Rect::from_min_max(a, b)))
            });
        if page != self.coach.session.page || outside_exercise {
            self.coach.session.page = page;
            self.coach.session.invalidate(Instant::now());
        } else {
            self.coach.session.changed(true, Instant::now());
        }
    }

    fn coach_label(&self) -> &'static str {
        if !self.coach_enabled() {
            return "AI off";
        }
        if self.lib.index.ai.provider == Provider::Gemini && self.coach.key.is_empty() {
            return "Add API key";
        }
        let s = &self.coach.session;
        if s.error.is_some() {
            return "Unavailable";
        }
        if s.result.is_some() && s.revision != s.evaluated {
            return "Updating";
        }
        if let Some(result) = &s.result {
            if !result.exercise_found {
                return "No exercise";
            }
            return result.verdict.label();
        }
        if s.busy.is_some() {
            "Reading"
        } else {
            "Waiting for ink"
        }
    }

    pub(super) fn ui_coach_button(&mut self, ui: &mut Ui) -> Response {
        let fill = if self.coach_enabled() {
            mix_col(self.look.paper, self.look.accent, 0.38)
        } else {
            self.look.paper
        };
        let button = self.round_well(ui, self.chrome_btn(), fill, paint_graduation_cap);
        button.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Exercise coach")
        });
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("coach-button-rect"), button.rect));
        if button.clone().on_hover_text("Exercise coach").clicked() {
            self.open_coach();
        }
        button
    }

    pub(super) fn ui_coach_meter(&mut self, ui: &mut Ui, rect: Rect) {
        let label = self.coach_label();
        let response = ui.interact(rect, Id::new("exercise-coach-meter"), Sense::click());
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, true, format!("Exercise coach: {label}"))
        });
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("coach-meter-rect"), rect));
        let painter = ui.painter();
        let track = Rect::from_center_size(rect.center(), vec2((rect.width() - 8.0).max(1.0), 4.0));
        painter.rect_filled(track, 2.0, mix_col(self.look.desk, self.look.fg, 0.18));
        let enabled = self.coach_enabled();
        let unavailable = self.coach.session.error.is_some()
            || (self.lib.index.ai.provider == Provider::Gemini && self.coach.key.is_empty());
        let verdict = self
            .coach
            .session
            .result
            .as_ref()
            .filter(|_| enabled && !unavailable)
            .map(|r| r.verdict)
            .unwrap_or(Verdict::Insufficient);
        let value = ui.ctx().animate_value_with_time(
            Id::new(("exercise-evaluation", self.coach.session.note)),
            verdict.position(),
            0.18,
        );
        let color = match verdict {
            Verdict::OnTrack => self.look.green(),
            Verdict::LikelyError => self.look.punch,
            Verdict::CheckStep => self.look.accent,
            Verdict::Insufficient => mix_col(self.look.desk, self.look.fg, 0.5),
        };
        if verdict != Verdict::Insufficient {
            painter.rect_filled(
                Rect::from_min_max(
                    track.min,
                    pos2(track.left() + track.width() * value, track.bottom()),
                ),
                2.0,
                color,
            );
        }
        if enabled {
            let marker = pos2(track.left() + track.width() * value, track.center().y);
            painter.circle_filled(marker, 4.0, self.look.desk);
            if self.coach.session.revision != self.coach.session.evaluated && !unavailable {
                painter.circle_stroke(marker, 2.5, Stroke::new(1.0, color));
            } else {
                painter.circle_filled(marker, 2.5, color);
            }
        }
        if response
            .on_hover_text(format!("{label} · Exercise coach settings and explanation"))
            .on_hover_cursor(CursorIcon::PointingHand)
            .clicked()
        {
            self.open_coach();
        }
    }

    pub(super) fn ui_coach(&mut self, ctx: &Context) {
        if !self.coach.open || self.note.is_none() {
            return;
        }
        let mut open = true;
        let screen = ctx.screen_rect();
        let panel_width = (screen.width() - 48.0).clamp(248.0, 344.0);
        let anchor = ctx
            .data(|d| d.get_temp::<Rect>(Id::new("coach-button-rect")))
            .unwrap_or_else(|| Rect::from_min_size(screen.left_top(), vec2(44.0, 44.0)));
        let panel_pos = pos2(
            (anchor.center().x - panel_width * 0.5)
                .clamp(screen.left() + 8.0, screen.right() - panel_width - 8.0),
            anchor.bottom() + 6.0,
        );
        let desk = self.look.desk;
        let surface = self.look.desk_deep;
        let edge = self.look.desk_edge;
        let fg = self.look.fg;
        let muted = mix_col(surface, fg, 0.68);
        let accent = self.look.accent;
        let now_enabled = self.coach_enabled();
        let status = self.coach_label();
        let status_color = if self.coach.session.error.is_some() {
            self.look.punch
        } else if let Some(result) = &self.coach.session.result {
            match result.verdict {
                Verdict::OnTrack => self.look.green(),
                Verdict::LikelyError => self.look.punch,
                Verdict::CheckStep => accent,
                Verdict::Insufficient => muted,
            }
        } else if now_enabled {
            accent
        } else {
            muted
        };

        Window::new(
            RichText::new("Exercise coach")
                .font(self.look.serif(18.0))
                .color(fg),
        )
        .id(Id::new("inkarchy-coach-settings"))
        .open(&mut open)
        .fixed_pos(panel_pos)
        .resizable(false)
        .collapsible(false)
        .vscroll(true)
        .min_width(panel_width)
        .max_width(panel_width)
        .default_width(panel_width)
        .default_height(420.0)
        .max_height((screen.height() - 56.0).max(120.0))
        .constrain_to(screen.shrink(8.0))
        .frame(
            Frame::NONE
                .fill(surface)
                .stroke(Stroke::new(1.0, edge))
                .corner_radius(12)
                .inner_margin(14),
        )
        .show(ctx, |ui| {
            ui.set_width(panel_width);
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            ui.spacing_mut().text_edit_width = panel_width;
            ui.style_mut().wrap_mode = Some(TextWrapMode::Wrap);

            let visuals = ui.visuals_mut();
            visuals.override_text_color = Some(fg);
            visuals.weak_text_color = Some(muted);
            visuals.extreme_bg_color = desk;
            visuals.code_bg_color = desk;
            visuals.selection.bg_fill = mix_col(desk, accent, 0.45);
            visuals.selection.stroke = Stroke::new(1.0, fg);
            for widget in [
                &mut visuals.widgets.noninteractive,
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
                &mut visuals.widgets.open,
            ] {
                widget.bg_fill = desk;
                widget.weak_bg_fill = desk;
                widget.fg_stroke = Stroke::new(1.0, fg);
                widget.bg_stroke = Stroke::new(0.7, edge);
            }
            visuals.widgets.hovered.bg_fill = mix_col(desk, accent, 0.22);
            visuals.widgets.active.bg_fill = mix_col(desk, accent, 0.38);

            let notebook_id = self.note.as_ref().unwrap().id;
            let mut enabled = now_enabled;
            if ui.checkbox(&mut enabled, "Enable in this notebook").changed() {
                if enabled {
                    self.lib.index.ai.enabled_notebooks.insert(notebook_id);
                } else {
                    self.lib.index.ai.enabled_notebooks.remove(&notebook_id);
                }
                self.coach.session.invalidate(Instant::now());
                let saved = self.lib.save_index();
                self.report_save(saved);
            }

            Frame::NONE
                .fill(desk)
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(status).color(status_color).strong());
                    if let Some(error) = &self.coach.session.error {
                        ui.label(RichText::new(error).small().color(self.look.punch));
                    }
                    if let Some(result) = &self.coach.session.result {
                        if !result.recognized_step.is_empty() {
                            ui.label(RichText::new("Read as").small().color(muted));
                            ui.label(RichText::new(&result.recognized_step).font(self.look.mono(14.0)));
                        }
                        if !result.explanation.is_empty() {
                            ui.label(RichText::new(&result.explanation).small());
                        }
                    }
                    if self.coach.session.error.is_some()
                        && ui
                            .add_enabled(enabled && self.coach.session.busy.is_none(), Button::new("Retry"))
                            .clicked()
                    {
                        self.coach.session.invalidate(Instant::now());
                    }
                });

            ui.horizontal(|ui| {
                ui.label(RichText::new("Statement").small().color(muted));
                let old_reference = self.coach.session.reference;
                let reference_label = old_reference
                    .map(|cell| {
                        self.note
                            .as_ref()
                            .unwrap()
                            .pages
                            .iter()
                            .position(|p| (p.col, p.row) == cell)
                            .map(|p| format!("Page {}", p + 1))
                            .unwrap_or_else(|| "Removed".into())
                    })
                    .unwrap_or_else(|| "This page".into());
                ComboBox::from_id_salt("coach-reference")
                    .selected_text(reference_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.coach.session.reference,
                            None,
                            "This page",
                        );
                        for (i, page) in self.note.as_ref().unwrap().pages.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.coach.session.reference,
                                Some((page.col, page.row)),
                                format!("Page {}", i + 1),
                            );
                        }
                    });
                if old_reference != self.coach.session.reference {
                    self.coach.session.invalidate(Instant::now());
                }
            });

            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.coach.draft.provider, Provider::Ollama, "Local");
                ui.selectable_value(&mut self.coach.draft.provider, Provider::Gemini, "Gemini API");
            });

            ui.collapsing("Model & connection", |ui| {
                ui.label(RichText::new("Shared by all notebooks").small().color(muted));
                match self.coach.draft.provider {
                    Provider::Ollama => {
                        ui.add(
                            TextEdit::singleline(&mut self.coach.draft.local_url)
                                .hint_text("Ollama address"),
                        );
                        ui.add(
                            TextEdit::singleline(&mut self.coach.draft.local_model)
                                .hint_text("Vision model"),
                        );
                        ui.label(
                            RichText::new("Install: ollama pull qwen3-vl:4b-instruct")
                                .small()
                                .monospace(),
                        );
                        ui.label(
                            RichText::new("CPU-only analysis may be slow.")
                                .small()
                                .color(muted),
                        );
                    }
                    Provider::Gemini => {
                        ui.add(
                            TextEdit::singleline(&mut self.coach.draft.api_model)
                                .hint_text("Model name"),
                        );
                        ui.add(
                            TextEdit::singleline(&mut self.coach.key_entry)
                                .password(true)
                                .hint_text("API key"),
                        );
                        ui.horizontal_wrapped(|ui| {
                            let has_key = !self.coach.key_entry.trim().is_empty();
                            if ui
                                .add_enabled(has_key, Button::new("Use for session"))
                                .clicked()
                            {
                                self.coach.key = self.coach.key_entry.trim().to_owned();
                                self.coach.key_entry.clear();
                                self.coach.keyring_attempted = true;
                                self.coach.keyring_status = "Key active for this session.".into();
                                self.coach.session.invalidate(Instant::now());
                            }
                            if ui
                                .add_enabled(has_key && self.coach.keyring.is_none(), Button::new("Save key"))
                                .clicked()
                            {
                                self.coach.key = self.coach.key_entry.trim().to_owned();
                                self.coach.key_entry.clear();
                                self.coach.keyring_attempted = true;
                                self.coach.keyring_action = 1;
                                self.coach.keyring = Some(CredentialJob::start(
                                    CredentialAction::Save(self.coach.key.clone()),
                                    ctx,
                                ));
                                self.coach.session.invalidate(Instant::now());
                            }
                            if !self.coach.key.is_empty() && ui.button("Forget").clicked() {
                                self.coach.key.clear();
                                self.coach.key_entry.clear();
                                self.coach.keyring_attempted = true;
                                self.coach.keyring_action = 2;
                                self.coach.keyring = Some(CredentialJob::start(
                                    CredentialAction::Delete,
                                    ctx,
                                ));
                                self.coach.session.invalidate(Instant::now());
                            }
                        });
                        if !self.coach.keyring_status.is_empty() {
                            ui.label(RichText::new(&self.coach.keyring_status).small().color(muted));
                        }
                        ui.label(
                            RichText::new("Enabled notebooks send page images to Google. API use may cost money.")
                                .small()
                                .color(muted),
                        );
                    }
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(self.coach.connection.is_none(), Button::new("Test"))
                        .clicked()
                    {
                        let settings = self.coach.draft.clone();
                        let key = if self.coach.key_entry.trim().is_empty() {
                            self.coach.key.clone()
                        } else {
                            self.coach.key_entry.trim().to_owned()
                        };
                        let ctx = ctx.clone();
                        let (tx, rx) = mpsc::channel();
                        self.coach.connection = Some(rx);
                        self.coach.connection_status = "Checking…".into();
                        let _ = std::thread::Builder::new()
                            .name("inkarchy-ai-check".into())
                            .spawn(move || {
                                let _ = tx.send(ai::check_connection(&settings, &key));
                                ctx.request_repaint();
                            });
                    }
                    if ui.button(RichText::new("Apply").strong()).clicked() {
                        self.coach.draft.enabled_notebooks =
                            self.lib.index.ai.enabled_notebooks.clone();
                        self.lib.index.ai = self.coach.draft.clone();
                        self.coach.session.invalidate(Instant::now());
                        let saved = self.lib.save_index();
                        self.report_save(saved);
                        self.coach.connection_status = "Settings applied.".into();
                    }
                });
                if !self.coach.connection_status.is_empty() {
                    ui.label(
                        RichText::new(&self.coach.connection_status)
                            .small()
                            .color(muted),
                    );
                }
            });

            if let Some(timings) = self.coach.session.timings {
                ui.collapsing("Timing", |ui| {
                    ui.label(RichText::new(format!(
                        "Load {:.1}s · image {:.0}ms · model {:.1}s",
                        timings.load.as_secs_f64(),
                        timings.prepare.as_secs_f64() * 1000.0,
                        timings.inference.as_secs_f64(),
                    )).small().color(muted));
                });
            }
        });
        self.coach.open = open;
    }
}
