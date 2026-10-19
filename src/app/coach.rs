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
        let screen = ctx.screen_rect();
        let panel_width = (screen.width() - 48.0).clamp(220.0, 276.0);
        let anchor = ctx.data(|d| d.get_temp::<Rect>(Id::new("coach-button-rect")))
            .unwrap_or_else(|| Rect::from_min_size(screen.left_top(), vec2(44.0, 44.0)));
        let panel_pos = pos2(
            (anchor.center().x - panel_width * 0.5).clamp(screen.left() + 8.0, screen.right() - panel_width - 8.0),
            anchor.bottom() + 6.0,
        );
        let fg = self.look.ink;
        let muted = fg.gamma_multiply(0.62);
        let notebook_id = self.note.as_ref().unwrap().id;
        let mut enabled = self.lib.index.ai.enabled_notebooks.contains(&notebook_id);
        let panel = Area::new(Id::new("inkarchy-coach-settings"))
            .fixed_pos(panel_pos).order(Order::Foreground).constrain(true)
            .show(ctx, |ui| {
                self.note_popup_frame()
                    .fill(self.look.paper)
                    .stroke(Stroke::new(1.0, fg.gamma_multiply(0.18)))
                    .show(ui, |ui| {
                        ui.set_width(panel_width - 16.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Exercise coach").font(self.look.serif(18.0)).color(fg));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let (rect, close) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                                let center = rect.center();
                                ui.painter().circle_stroke(center, 12.0, Stroke::new(1.0, muted));
                                for (a, b) in [(vec2(-3.0, -3.0), vec2(3.0, 3.0)),
                                    (vec2(3.0, -3.0), vec2(-3.0, 3.0))] {
                                    ui.painter().line_segment([center + a, center + b], Stroke::new(1.5, fg));
                                }
                                if close.on_hover_text("Close").on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    self.coach.open = false;
                                }
                            });
                        });
                        let (rect, toggle) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                        toggle.widget_info(|| WidgetInfo::selected(
                            WidgetType::Button, ui.is_enabled(), enabled, "Enable for this notebook"
                        ));
                        let fill = if enabled { mix_col(self.look.paper, self.look.green(), 0.28) }
                            else { self.look.paper };
                        let (face, ink) = self.paint_note_surface(ui, rect.shrink(2.0), &toggle, fill, 10);
                        let swatch = Rect::from_min_size(face.min + vec2(8.0, 6.0), vec2(58.0, 40.0));
                        self.paint_paper_swatch(ui.painter(), swatch, PaperKind::Lined);
                        ui.painter().circle_filled(swatch.right_top() - vec2(7.0, -7.0), 3.2,
                            if enabled { self.look.green() } else { muted });
                        ui.painter().text(pos2(swatch.right() + 12.0, face.center().y),
                            Align2::LEFT_CENTER, "Enable AI", self.look.serif(16.0), ink);
                        if toggle.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            enabled = !enabled;
                            if enabled { self.lib.index.ai.enabled_notebooks.insert(notebook_id); }
                            else { self.lib.index.ai.enabled_notebooks.remove(&notebook_id); }
                            self.coach.session.invalidate(Instant::now());
                            let saved = self.lib.save_index();
                            self.report_save(saved);
                        }
                        ui.add_space(7.0);
                        ui.label(RichText::new("Connections").font(self.look.serif(15.0)).color(muted));
                        for (provider, title, mark) in [
                                (Provider::Ollama, "Ollama · local", "O"),
                                (Provider::Gemini, "Gemini · cloud", "G"),
                            ] {
                                let selected = self.lib.index.ai.provider == provider;
                                let color = if selected && !self.coach.connection_status.is_empty() {
                                    if self.coach.connection_status.contains("ready") || self.coach.connection_status.contains("connected") { self.look.green() } else { self.look.punch }
                                } else { muted };
                                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
                                let badge = pos2(rect.left() + 19.0, rect.center().y);
                                ui.painter().circle_filled(badge, 12.0, mix_col(self.look.paper, color, 0.18));
                                ui.painter().circle_stroke(badge, 12.0, Stroke::new(1.0, color.gamma_multiply(0.5)));
                                ui.painter().text(badge, Align2::CENTER_CENTER, mark, self.look.mono(13.0), fg);
                                ui.painter().text(pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER,
                                    title, self.look.mono(12.0), if selected { fg } else { muted });
                                ui.painter().circle_filled(pos2(rect.right() - 12.0, rect.center().y), 3.0, color);
                        }
                        let checking = self.coach.connection.is_some();
                        let (rect, check) = ui.allocate_exact_size(
                            vec2(ui.available_width(), 44.0),
                            if checking { Sense::hover() } else { Sense::click() },
                        );
                        check.widget_info(|| {
                            WidgetInfo::labeled(WidgetType::Button, !checking, "Test AI connection")
                        });
                        let face_fill = mix_col(self.look.paper, self.look.accent, 0.10);
                        let (face, ink) = self.paint_note_surface(
                            ui, rect.shrink2(vec2(1.0, 2.0)), &check, face_fill, 11,
                        );
                        let seal = pos2(face.left() + 22.0, face.center().y);
                        let painter = ui.painter();
                        painter.circle_filled(seal, 12.0, mix_col(face_fill, self.look.accent, 0.24));
                        painter.circle_stroke(seal, 12.0, Stroke::new(0.9, ink.gamma_multiply(0.36)));
                        painter.line_segment(
                            [seal + vec2(-4.0, 3.0), seal + vec2(4.0, -3.0)],
                            Stroke::new(1.6, ink),
                        );
                        painter.circle_filled(seal + vec2(-4.0, 3.0), 2.1, ink);
                        painter.circle_filled(seal + vec2(4.0, -3.0), 2.1, ink);
                        painter.text(
                            pos2(face.left() + 42.0, face.center().y),
                            Align2::LEFT_CENTER,
                            if checking { "Checking…" } else { "Test connection" },
                            self.look.serif(15.0), ink,
                        );
                        if !checking && check.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            let settings = self.lib.index.ai.clone();
                            let key = self.coach.key.clone();
                            let ctx = ctx.clone();
                            let (tx, rx) = mpsc::channel();
                            self.coach.connection = Some(rx);
                            self.coach.connection_status = "Checking…".into();
                            let _ = std::thread::Builder::new().name("inkarchy-ai-check".into()).spawn(move || {
                                let _ = tx.send(ai::check_connection(&settings, &key));
                                ctx.request_repaint();
                            });
                        }
                        if !checking && !self.coach.connection_status.is_empty()
                            && self.coach.connection_status != "Checking…" {
                            ui.label(RichText::new(&self.coach.connection_status).small().color(muted));
                        }
                    });
            });
        let clicked_outside = ctx.input(|input| input.pointer.primary_pressed()
            && input.pointer.interact_pos().is_some_and(|pos| !panel.response.rect.contains(pos) && !anchor.contains(pos)));
        if clicked_outside || ctx.input(|input| input.key_pressed(Key::Escape)) { self.coach.open = false; }
    }
}
