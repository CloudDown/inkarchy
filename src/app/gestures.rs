//! Finger gestures belong to the canvas from contact down to contact up.
//! Keep them independent of the synthetic mouse pointer (and stylus hover).
use super::*;

const TOUCH_SLOP: f32 = 8.0;
const HOLD_TIME: f64 = 0.38;

#[derive(Clone, Copy)]
enum CanvasGesture {
    Pan(Option<Sel>),
    Move,
    Resize {
        anchor: Pos2,
        initial: Vec2,
        scale: f32,
    },
    /// One finger on the sticky's corner. `start` is the size at contact.
    StickySize {
        page: usize,
        id: Uuid,
        start: Vec2,
    },
    Text,
    Tool,
}

pub(super) struct TouchGesture {
    device: TouchDeviceId,
    id: TouchId,
    origin: Pos2,
    last: Pos2,
    since: f64,
    mode: CanvasGesture,
    moved: bool,
    snapshot: bool,
}

impl InkarchyApp {
    /// A short hold opens settings once; lifting that finger must not also
    /// toggle the tool. Mouse right-click and keyboard clicks stay available.
    pub(super) fn tool_button_action(response: &Response) -> (bool, bool) {
        let ctx = &response.ctx;
        let key = response.id.with("settings-hold");
        let (pressed, down, touching, held, still) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.any_touches() && i.multi_touch().is_none(),
                i.pointer.press_start_time().map_or(0.0, |t| i.time - t),
                i.pointer
                    .press_origin()
                    .zip(i.pointer.interact_pos())
                    .is_some_and(|(a, b)| a.distance(b) <= TOUCH_SLOP),
            )
        });
        if pressed {
            ctx.data_mut(|d| d.remove::<bool>(key));
        }
        let already_held = ctx.data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
        let holding = touching && down && still && response.is_pointer_button_down_on();
        let open = !already_held
            && ((holding && held >= HOLD_TIME) || response.clicked_by(PointerButton::Secondary));
        if holding && !already_held && !open {
            ctx.request_repaint_after(Duration::from_secs_f64((HOLD_TIME - held).max(0.001)));
        }
        if open && touching {
            ctx.data_mut(|d| d.insert_temp(key, true));
        }
        let click = response.clicked() && !already_held && !open;
        if !down {
            ctx.data_mut(|d| d.remove::<bool>(key));
        }
        (click, open)
    }

    pub(super) fn reset_canvas_gesture(&mut self) {
        self.canvas_touch = None;
        self.touch_navigation = false;
        self.drag_last = None;
        self.selection_resize = None;
        self.lasso.clear();
    }

    /// Returns true when the touch is navigation/object manipulation rather
    /// than input for the currently selected drawing tool.
    pub(super) fn handle_canvas_touch(&mut self, ui: &Ui, canvas: Rect) -> bool {
        let (events, touching, multiple, time) = ui.input(|i| {
            let events: Vec<_> = i
                .events
                .iter()
                .filter_map(|event| match *event {
                    Event::Touch {
                        device_id,
                        id,
                        phase,
                        pos,
                        ..
                    } => Some((device_id, id, phase, pos)),
                    _ => None,
                })
                .collect();
            (events, i.any_touches(), i.multi_touch().is_some(), i.time)
        });
        if events.is_empty() && !touching && self.canvas_touch.is_none() {
            self.touch_navigation = false;
            return false;
        }
        if self.sticky_sheet.is_some() {
            self.canvas_touch = None;
            return true;
        }
        let pen = self.tablet.snapshot();
        if !ui.input(|i| i.focused) {
            self.reset_canvas_gesture();
            if !self.live_from_pen {
                self.live = None;
                self.clear_shape_hold();
            }
            return true;
        }
        if multiple || self.touch_navigation {
            self.canvas_touch = None;
            self.drag_last = None;
            self.selection_resize = None;
            self.lasso.clear();
            if !self.live_from_pen {
                self.live = None;
                self.clear_shape_hold();
            }
            self.touch_navigation = touching;
            return true;
        }
        if self.sticky_wheel.is_some() {
            return true;
        }
        if self.export_picker_open || Popup::is_any_open(ui.ctx()) || self.palm_guard(&pen) {
            self.canvas_touch = None;
            self.drag_last = None;
            self.selection_resize = None;
            return true;
        }

        let mut tool_input = self
            .canvas_touch
            .as_ref()
            .is_some_and(|g| matches!(g.mode, CanvasGesture::Tool));
        // A press that began on a + or − stays with Pages mode, including the drag that cancels it.
        if self.page_press_owned {
            return false;
        }
        for (device, id, phase, screen) in events {
            if phase == TouchPhase::Start && self.canvas_touch.is_none() {
                if !canvas.contains(screen) || Self::pen_over_chrome(ui.ctx(), screen) {
                    continue;
                }
                if self.pages_marks_ready
                    && (self.sheet_tab_at(screen, canvas).is_some()
                        || self.page_minus_at(screen, canvas).is_some()
                        || self.sticky_minus_at(screen, canvas).is_some())
                {
                    return false; // Page marks own their press.
                }
                let paper = self.camera.to_paper(screen, canvas);
                if self.editing_text.is_some_and(|(page, id)| {
                    self.note
                        .as_ref()
                        .and_then(|n| n.pages.get(page))
                        .and_then(|p| p.texts.iter().find(|t| t.id == id))
                        .is_some_and(|t| t.rect().translate(self.origin_of(page)).contains(paper))
                }) {
                    continue; // TextEdit owns caret placement and text selection.
                }
                let hit = self.hit_object(paper);
                let mode = if self.sheet_tabs_on {
                    CanvasGesture::Pan(None)
                } else if self.tool.is_eraser()
                    || (self.lib.index.finger_draw && self.tool.is_ink())
                {
                    self.sel.clear();
                    CanvasGesture::Tool
                } else if let Some((page, id, start)) = self.sticky_corner(screen, canvas) {
                    self.finish_text_edit();
                    self.sel.clear();
                    CanvasGesture::StickySize { page, id, start }
                } else if let Some((corner, anchor)) = self.selection_corner(screen, canvas) {
                    CanvasGesture::Resize {
                        anchor,
                        initial: corner - anchor,
                        scale: 1.0,
                    }
                } else if self.selection_bounds().is_some_and(|b| b.contains(paper)) {
                    self.finish_text_edit();
                    CanvasGesture::Move
                } else if let Some(hit) =
                    hit.filter(|s| s.kind != SelKind::Stroke || self.tool == Tool::Lasso)
                {
                    self.finish_text_edit();
                    self.sel = vec![hit];
                    CanvasGesture::Move
                } else if self.tool == Tool::Lasso {
                    CanvasGesture::Tool
                } else if self.tool == Tool::Text {
                    CanvasGesture::Text
                } else {
                    CanvasGesture::Pan(hit)
                };
                tool_input = matches!(mode, CanvasGesture::Tool);
                self.canvas_touch = Some(TouchGesture {
                    device,
                    id,
                    origin: screen,
                    last: screen,
                    since: time,
                    mode,
                    moved: false,
                    snapshot: false,
                });
            }
            let Some(mut gesture) = self.canvas_touch.take() else {
                continue;
            };
            if gesture.device != device || gesture.id != id {
                self.canvas_touch = Some(gesture);
                continue;
            }
            if phase == TouchPhase::Cancel {
                self.drag_last = None;
                self.selection_resize = None;
                self.lasso.clear();
                if !self.live_from_pen {
                    self.live = None;
                    self.clear_shape_hold();
                }
                tool_input = false;
                continue;
            }
            let first_move = !gesture.moved && screen.distance(gesture.origin) > TOUCH_SLOP;
            gesture.moved |= first_move;
            if gesture.moved {
                let previous = if first_move {
                    gesture.origin
                } else {
                    gesture.last
                };
                let delta = self.camera.screen_to_paper_delta(screen - previous);
                match gesture.mode {
                    CanvasGesture::Move => {
                        if delta != Vec2::ZERO {
                            if !gesture.snapshot {
                                self.push_snapshot();
                                gesture.snapshot = true;
                            }
                            self.translate_sel(delta);
                            self.drag_last = Some(self.camera.to_paper(screen, canvas));
                            self.mark_dirty();
                        }
                    }
                    CanvasGesture::Resize {
                        anchor,
                        initial,
                        scale,
                    } => {
                        // Offset the handle by finger travel: a generous hit area
                        // must not make the image jump when grabbed off-centre.
                        let travel = self.camera.screen_to_paper_delta(screen - gesture.origin);
                        let next = ((initial + travel).dot(initial) / initial.length_sq().max(1.0))
                            .clamp(0.1, 10.0);
                        if (next - scale).abs() > 0.0001 {
                            if !gesture.snapshot {
                                self.push_snapshot();
                                gesture.snapshot = true;
                            }
                            self.scale_selection(anchor, next / scale);
                            gesture.mode = CanvasGesture::Resize {
                                anchor,
                                initial,
                                scale: next,
                            };
                            self.selection_resize = Some((anchor, initial, next));
                            self.mark_dirty();
                        }
                    }
                    CanvasGesture::Pan(_) | CanvasGesture::Text => {
                        self.camera.pan += screen - previous;
                        self.fitted_cell = None;
                    }
                    CanvasGesture::StickySize { page, id, start } => {
                        let travel = self.camera.screen_to_paper_delta(screen - gesture.origin);
                        let size = vec2(
                            (start.x + travel.x).max(Self::STICKY_MIN),
                            (start.y + travel.y).max(Self::STICKY_MIN),
                        );
                        if size != start {
                            if !gesture.snapshot {
                                self.push_snapshot();
                                gesture.snapshot = true;
                            }
                            self.set_sticky_size(page, id, size);
                            self.mark_dirty();
                        }
                    }
                    CanvasGesture::Tool => {}
                }
            }
            gesture.last = screen;
            if phase == TouchPhase::End {
                self.drag_last = None;
                self.selection_resize = None;
                if !gesture.moved {
                    match gesture.mode {
                        CanvasGesture::Text => {
                            self.activate_text(self.camera.to_paper(screen, canvas))
                        }
                        CanvasGesture::Pan(_) => {
                            self.finish_text_edit();
                            self.sel.clear();
                        }
                        CanvasGesture::Move => {
                            let double = ui
                                .input(|i| i.pointer.button_double_clicked(PointerButton::Primary));
                            if let Some((page, id)) = self.selected_sticky() {
                                if double && time - gesture.since < super::WHEEL_HOLD {
                                    self.open_sticky_sheet(page, id);
                                }
                            } else if double {
                                self.edit_selected_text();
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                self.canvas_touch = Some(gesture);
            }
        }

        // A still finger on a sticky opens the folder cloth wheel.
        let sticky_hold = self.canvas_touch.as_ref().and_then(|gesture| {
            (!gesture.moved && matches!(gesture.mode, CanvasGesture::Move))
                .then_some((gesture.origin, gesture.since))
        });
        if let Some((origin, since)) = sticky_hold {
            if let Some((page, id)) = self.selected_sticky() {
                let held = time - since;
                if held >= super::WHEEL_HOLD {
                    self.canvas_touch = None;
                    self.drag_last = None;
                    self.open_sticky_wheel(page, id, origin, ui.ctx(), false);
                } else {
                    ui.ctx().request_repaint_after(Duration::from_secs_f64(
                        (super::WHEEL_HOLD - held).max(0.001),
                    ));
                }
            }
        }

        // Holding over ink selects the stroke; moving before the threshold pans.
        if let Some(gesture) = &mut self.canvas_touch {
            if let CanvasGesture::Pan(Some(hit)) = gesture.mode {
                if !gesture.moved {
                    if time - gesture.since >= HOLD_TIME {
                        gesture.mode = CanvasGesture::Move;
                        self.sel = vec![hit];
                        self.finish_text_edit();
                    } else {
                        ui.ctx().request_repaint_after(Duration::from_millis(16));
                    }
                }
            }
        }
        !tool_input
    }

    pub(super) fn paint_touch_hold(&self, ui: &Ui, painter: &Painter) {
        let Some(g) = &self.canvas_touch else { return };
        if !g.moved && matches!(g.mode, CanvasGesture::Pan(Some(_))) {
            let progress = ((ui.input(|i| i.time) - g.since) / HOLD_TIME).clamp(0.0, 1.0) as f32;
            if progress < 0.2 {
                return;
            }
            let points = (0..=24)
                .map(|i| {
                    let angle = -std::f32::consts::FRAC_PI_2
                        + std::f32::consts::TAU * progress * i as f32 / 24.0;
                    g.origin + Vec2::angled(angle) * 24.0
                })
                .collect();
            painter.add(Shape::line(points, Stroke::new(2.0, self.look.accent)));
        }
    }

    pub(super) fn edit_selected_text(&mut self) {
        if let [Sel {
            page,
            id,
            kind: SelKind::Text,
        }] = self.sel.as_slice()
        {
            self.editing_text = Some((*page, *id));
            self.text_focus = true;
            self.tool = Tool::Text;
            self.sel.clear();
        }
    }
}
