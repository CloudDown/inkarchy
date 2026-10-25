use super::*;
use crate::ai::{self, CredentialAction, CredentialJob, Provider, Session, Verdict, Worker};

/// Hide the writing coach chrome and background analysis until it is ready again.
pub(crate) const COACH_UI_VISIBLE: bool = false;

#[derive(Default)]
pub(super) struct CoachUi {
    pub session: Session,
    worker: Option<Worker>,
    pub(super) open: bool,
    key: String,
    keyring: Option<CredentialJob>,
    keyring_attempted: bool,
}

impl InkarchyApp {
    pub(super) fn coach_enabled(&self) -> bool {
        COACH_UI_VISIBLE
            && self
                .note
                .as_ref()
                .is_some_and(|n| self.lib.index.ai.enabled_notebooks.contains(&n.id))
    }

    pub(super) fn open_coach(&mut self) {
        if !COACH_UI_VISIBLE {
            return;
        }
        self.coach.open = true;
    }

    pub(super) fn tick_coach(&mut self, ctx: &Context) {
        if !COACH_UI_VISIBLE {
            self.coach.open = false;
            return;
        }
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
            if let Ok(value) = done {
                self.coach.key = value;
                self.coach.session.invalidate(Instant::now());
            }
        }
        let uses_api = (self.coach_enabled() || self.coach.open)
            && self.lib.index.ai.provider == Provider::Gemini;
        if uses_api && self.coach.key.is_empty() && !self.coach.keyring_attempted {
            self.coach.keyring_attempted = true;
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
            return "Coach off";
        }
        if self.lib.index.ai.provider == Provider::Gemini && self.coach.key.is_empty() {
            return "Add API key";
        }
        let s = &self.coach.session;
        if s.error.is_some() {
            return "Needs attention";
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
        if !COACH_UI_VISIBLE {
            return ui.allocate_response(Vec2::ZERO, Sense::hover());
        }
        // Only the raised cap is visible; the well matches other dock chrome.
        let slot = self.slot();
        let (rect, button) = ui.allocate_exact_size(vec2(slot, slot), Sense::click());
        let center = rect.center() - vec2(0.0, if button.hovered() { 1.0 } else { 0.0 });
        paint_coach_icon(
            ui.painter(),
            center,
            self.look.ink,
            self.look.accent,
            self.look.paper,
            self.look.shadow,
            self.coach_enabled(),
        );
        button.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Writing coach")
        });
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("coach-button-rect"), button.rect));
        if button
            .clone()
            .on_hover_text(format!("Writing coach · {}", self.coach_label()))
            .clicked()
        {
            self.open_coach();
        }
        button
    }

    pub(super) fn ui_coach_meter(&mut self, ui: &mut Ui, rect: Rect) {
        if !COACH_UI_VISIBLE {
            return;
        }
        let label = self.coach_label();
        let response = ui.interact(rect, Id::new("exercise-coach-meter"), Sense::click());
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, true, format!("Exercise coach: {label}"))
        });
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("coach-meter-rect"), rect));
        let painter = ui.painter();
        let track_y = if rect.width() >= 124.0 {
            rect.center().y + 10.0
        } else {
            rect.center().y
        };
        let track = Rect::from_center_size(
            pos2(rect.center().x, track_y),
            vec2((rect.width() - 8.0).max(1.0), 4.0),
        );
        if rect.width() >= 124.0 {
            painter.text(
                pos2(rect.center().x, rect.center().y - 7.0),
                Align2::CENTER_CENTER,
                label,
                self.look.mono(11.0),
                self.look.fg.gamma_multiply(0.85),
            );
        }
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
            .on_hover_text(format!("Writing coach · {label}"))
            .on_hover_cursor(CursorIcon::PointingHand)
            .clicked()
        {
            self.open_coach();
        }
    }

    pub(super) fn ui_coach(&mut self, ctx: &Context) {
        if !COACH_UI_VISIBLE || !self.coach.open || self.note.is_none() {
            return;
        }
        let screen = ctx.screen_rect();
        let panel_width = (screen.width() - 48.0).clamp(220.0, 276.0);
        let anchor = ctx
            .data(|d| d.get_temp::<Rect>(Id::new("coach-button-rect")))
            .unwrap_or_else(|| Rect::from_min_size(screen.left_top(), vec2(44.0, 44.0)));
        let panel_pos = pos2(
            (anchor.center().x - panel_width * 0.5)
                .clamp(screen.left() + 8.0, screen.right() - panel_width - 8.0),
            if anchor.bottom() + 200.0 > screen.bottom() {
                (anchor.top() - 200.0 - 6.0).max(screen.top() + 8.0)
            } else {
                anchor.bottom() + 6.0
            },
        );
        let fg = self.look.ink;
        let notebook_id = self.note.as_ref().unwrap().id;
        let enabled = self.coach_enabled();
        let panel = Area::new(Id::new("inkarchy-coach-settings"))
            .fixed_pos(panel_pos)
            .order(Order::Foreground)
            .constrain(true)
            .show(ctx, |ui| {
                self.note_popup_frame()
                    .fill(self.look.paper)
                    .stroke(Stroke::new(1.0, fg.gamma_multiply(0.18)))
                    .show(ui, |ui| {
                        ui.set_width(panel_width - 16.0);
                        ui.label(
                            RichText::new("Writing coach")
                                .font(self.look.serif(18.0))
                                .color(fg),
                        );
                        ui.add_space(5.0);
                        let (rect, toggle) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                        toggle.widget_info(|| {
                            WidgetInfo::selected(
                                WidgetType::Button,
                                true,
                                enabled,
                                "Coach this notebook",
                            )
                        });
                        let fill = if enabled {
                            mix_col(self.look.paper, self.look.green(), 0.28)
                        } else {
                            self.look.paper
                        };
                        let (face, ink) =
                            self.paint_note_surface(ui, rect.shrink(2.0), &toggle, fill, 10);
                        let swatch =
                            Rect::from_min_size(face.min + vec2(8.0, 6.0), vec2(58.0, 40.0));
                        self.paint_paper_swatch(ui.painter(), swatch, PaperKind::Lined);
                        ui.painter().circle_filled(
                            swatch.right_top() - vec2(7.0, -7.0),
                            3.2,
                            if enabled {
                                self.look.green()
                            } else {
                                fg.gamma_multiply(0.55)
                            },
                        );
                        ui.painter().text(
                            pos2(swatch.right() + 12.0, face.center().y),
                            Align2::LEFT_CENTER,
                            "Coach this book",
                            self.look.serif(15.0),
                            ink,
                        );
                        if toggle.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            if enabled {
                                self.lib.index.ai.enabled_notebooks.remove(&notebook_id);
                            } else {
                                self.lib.index.ai.enabled_notebooks.insert(notebook_id);
                            }
                            self.coach.session.invalidate(Instant::now());
                            let saved = self.lib.save_index();
                            self.report_save(saved);
                        }
                        if self.lib.index.ai.provider == Provider::Gemini {
                            ui.label(
                                RichText::new("When on, page images are sent to Google.")
                                    .font(self.look.serif(13.0))
                                    .color(fg.gamma_multiply(0.7)),
                            );
                        }
                    });
            });
        let clicked_outside = ctx.input(|input| {
            input.pointer.primary_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| !panel.response.rect.contains(pos) && !anchor.contains(pos))
        });
        if clicked_outside || ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.coach.open = false;
        }
    }
}
