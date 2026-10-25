//! Pages mode is a workbench: a drafting gauge, loose paper stock and a return tab.
//! Objects have separate hit regions so the empty desk remains draggable.
use super::*;

impl InkarchyApp {
    pub(super) fn ui_atelier_exit(
        &mut self,
        ctx: &Context,
        compact: bool,
        write_anchor: Rect,
    ) -> Vec<Rect> {
        let canvas = self.canvas_rect;
        let mut objects = Vec::new();
        let toolbar = if compact {
            Rect::from_min_size(
                pos2(canvas.center().x - 150.0, canvas.bottom() - 56.0),
                vec2(300.0, 48.0),
            )
        } else {
            Rect::from_min_size(
                write_anchor.right_top() + vec2(-88.0, -32.0),
                vec2(88.0, 32.0),
            )
        };
        Area::new(Id::new("pages-atelier-return"))
            .fixed_pos(toolbar.min)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                let choices: &[&str] = if compact {
                    &["Arrange", "Paper", "Write"]
                } else {
                    &["Write"]
                };
                for (index, &name) in choices.iter().enumerate() {
                    let cell = Rect::from_min_size(
                        toolbar.min + vec2(index as f32 * 100.0, 0.0),
                        if compact {
                            vec2(96.0, 48.0)
                        } else {
                            toolbar.size()
                        },
                    );
                    let on = ctx.data(|d| {
                        d.get_temp::<bool>(Id::new("pages-material-open"))
                            .unwrap_or(false)
                    });
                    let selected =
                        compact && ((name == "Paper" && on) || (name == "Arrange" && !on));
                    let response = ui.interact(cell, ui.id().with(name), Sense::click());
                    response.widget_info(|| {
                        WidgetInfo::selected(WidgetType::Button, true, selected, name)
                    });
                    let p = ui.painter();
                    let face = cell.shrink2(vec2(1.0, 2.0)).translate(vec2(
                        0.0,
                        if response.is_pointer_button_down_on() {
                            2.0
                        } else {
                            0.0
                        },
                    ));
                    let corners = vec![
                        face.left_top(),
                        face.right_top(),
                        face.right_bottom() - vec2(9.0, 0.0),
                        face.left_bottom(),
                        face.left_top(),
                    ];
                    p.add(Shape::convex_polygon(
                        corners,
                        if selected || response.hovered() {
                            mix_col(self.look.paper, self.look.accent, 0.18)
                        } else {
                            self.look.paper
                        },
                        Stroke::new(1.0, self.look.ink.gamma_multiply(0.18)),
                    ));
                    if selected || response.has_focus() {
                        p.hline(
                            (face.left() + 12.0)..=(face.right() - 12.0),
                            face.bottom() - 6.0,
                            Stroke::new(2.0, self.look.accent),
                        );
                    }
                    p.text(
                        face.center(),
                        Align2::CENTER_CENTER,
                        name,
                        self.look.serif(18.0),
                        self.look.ink,
                    );
                    let key = if name == "Write" {
                        Id::new(("strip-tray", "Close"))
                    } else {
                        Id::new(("pages-atelier-tab", name))
                    };
                    ctx.data_mut(|d| d.insert_temp(key, cell));
                    if response
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(if name == "Write" {
                            "Return to writing · Esc"
                        } else {
                            "Bring this instrument forward"
                        })
                        .clicked()
                    {
                        if name == "Write" {
                            self.leave_pages_mode();
                        } else {
                            ctx.data_mut(|d| {
                                d.insert_temp(Id::new("pages-material-open"), name == "Paper")
                            });
                        }
                    }
                    objects.push(cell);
                }
            });
        objects
    }

    pub(super) fn paint_layout_instrument(&self, p: &Painter, rect: Rect) {
        // A squared drafting sheet with a calibrated edge, not a rounded toolbar.
        p.rect_filled(
            rect.translate(vec2(2.0, 5.0)),
            2.0,
            self.look.shadow.gamma_multiply(0.3),
        );
        p.rect_filled(rect, 2.0, self.look.paper);
        p.rect_stroke(
            rect.shrink(0.5),
            2.0,
            Stroke::new(1.0, self.look.ink.gamma_multiply(0.25)),
            StrokeKind::Inside,
        );
        let blue = self.look.accent.gamma_multiply(0.2);
        for i in 1..30 {
            let x = rect.left() + i as f32 * 10.0;
            p.vline(
                x,
                (rect.bottom() - if i % 5 == 0 { 11.0 } else { 5.0 })..=rect.bottom(),
                Stroke::new(1.0, self.look.ink.gamma_multiply(0.5)),
            );
        }
        p.text(
            rect.min + vec2(16.0, 15.0),
            Align2::LEFT_TOP,
            "Composition",
            self.look.serif(21.0),
            self.look.ink,
        );
        let count = self.note.as_ref().map_or(0, |n| n.pages.len());
        p.text(
            rect.min + vec2(16.0, 44.0),
            Align2::LEFT_TOP,
            if count == 1 {
                "1 SHEET".to_owned()
            } else {
                format!("{count} SHEETS")
            },
            self.look.mono(9.0),
            self.look.ink.gamma_multiply(0.55),
        );
        p.hline(
            (rect.left() + 14.0)..=(rect.right() - 14.0),
            rect.top() + 81.0,
            Stroke::new(1.0, blue),
        );
        p.text(
            rect.min + vec2(16.0, 66.0),
            Align2::LEFT_CENTER,
            "Drag the number to arrange",
            self.look.mono(10.0),
            self.look.ink.gamma_multiply(0.65),
        );
    }

    pub(super) fn paint_stock_caption(&self, p: &Painter, rect: Rect) {
        let label = self.note.as_ref().map_or("Blank", |n| n.paper.label());
        // A stock label tucked below the loose fan.
        let ticket = Rect::from_min_size(rect.min + vec2(8.0, 132.0), vec2(154.0, 52.0));
        p.rect_filled(
            ticket.translate(vec2(1.0, 3.0)),
            1.0,
            self.look.shadow.gamma_multiply(0.25),
        );
        p.rect_filled(ticket, 1.0, self.look.paper);
        p.text(
            ticket.min + vec2(10.0, 7.0),
            Align2::LEFT_TOP,
            label,
            self.look.serif(19.0),
            self.look.ink,
        );
        p.text(
            ticket.min + vec2(10.0, 33.0),
            Align2::LEFT_TOP,
            "PAPER · WHOLE BOOK",
            self.look.mono(9.0),
            self.look.ink.gamma_multiply(0.6),
        );
    }

    pub(super) fn paint_stock_leaf(
        &self,
        p: &Painter,
        hit: Rect,
        index: usize,
        kind: PaperKind,
        selected: bool,
        hot: bool,
    ) {
        let angle = (index as f32 - 2.5) * 0.075;
        let centre = hit.center()
            + vec2(
                0.0,
                angle.abs() * 20.0 - if hot || selected { 10.0 } else { 0.0 },
            );
        let point = |x: f32, y: f32| {
            centre
                + vec2(
                    x * angle.cos() - y * angle.sin(),
                    x * angle.sin() + y * angle.cos(),
                )
        };
        let corners = vec![
            point(-30.0, -46.0),
            point(30.0, -46.0),
            point(30.0, 46.0),
            point(-30.0, 46.0),
        ];
        p.add(Shape::convex_polygon(
            corners.iter().map(|v| *v + vec2(2.0, 5.0)).collect(),
            self.look.shadow.gamma_multiply(0.35),
            Stroke::NONE,
        ));
        p.add(Shape::convex_polygon(
            corners,
            self.look.paper,
            Stroke::new(
                if hot || selected { 1.5 } else { 0.7 },
                if hot || selected {
                    self.look.accent
                } else {
                    self.look.ink.gamma_multiply(0.3)
                },
            ),
        ));
        let rule = self.look.ink.gamma_multiply(0.22);
        let line = |a: (f32, f32), b: (f32, f32), color: Color32| {
            p.line_segment([point(a.0, a.1), point(b.0, b.1)], Stroke::new(0.65, color));
        };
        match kind {
            PaperKind::Blank => {}
            PaperKind::Dots => {
                for x in 0..5 {
                    for y in 0..8 {
                        p.circle_filled(
                            point(-20.0 + x as f32 * 10.0, -32.0 + y as f32 * 9.0),
                            1.1,
                            rule,
                        );
                    }
                }
            }
            _ => {
                let step = if kind == PaperKind::Millimetre {
                    4.0
                } else {
                    9.0
                };
                let mut y = -32.0;
                while y < 36.0 {
                    line((-23.0, y), (23.0, y), rule);
                    y += step;
                }
                if matches!(kind, PaperKind::Grid | PaperKind::Millimetre) {
                    let mut x = -23.0;
                    while x <= 23.0 {
                        line((x, -36.0), (x, 36.0), rule);
                        x += step;
                    }
                } else {
                    line(
                        (-12.0, -37.0),
                        (-12.0, 36.0),
                        self.look.rust().gamma_multiply(0.6),
                    );
                    if kind == PaperKind::Cornell {
                        line(
                            (-23.0, 25.0),
                            (23.0, 25.0),
                            self.look.rust().gamma_multiply(0.6),
                        );
                    }
                }
            }
        }
        if selected {
            paint_check(p, point(0.0, 37.0), self.look.ink);
        }
    }

    pub(super) fn paint_stock_tone(
        &self,
        p: &Painter,
        cell: Rect,
        tone: PaperTone,
        selected: bool,
        hot: bool,
    ) {
        let face = Rect::from_center_size(cell.center(), vec2(30.0, 36.0));
        p.rect_filled(
            face.translate(vec2(2.0, 3.0)),
            1.0,
            self.look.shadow.gamma_multiply(0.3),
        );
        let fill = self.look.paper_surface(tone);
        p.rect_filled(face, 1.0, fill);
        p.rect_stroke(
            face,
            1.0,
            Stroke::new(
                if selected || hot { 1.8 } else { 0.8 },
                if selected || hot {
                    self.look.accent
                } else {
                    self.look.ink.gamma_multiply(0.35)
                },
            ),
            StrokeKind::Inside,
        );
        if selected {
            paint_check(
                p,
                face.center(),
                well_glyph(fill, self.look.paper, self.look.ink),
            );
        }
    }
}
