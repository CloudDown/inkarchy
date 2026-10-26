//! Pages mode is a workbench: a drafting gauge and loose paper stock.
//! Objects have separate hit regions so the empty desk remains draggable.
use super::*;

impl InkarchyApp {
    pub(super) fn ui_atelier_exit(&mut self, ctx: &Context, compact: bool) -> Vec<Rect> {
        if !compact {
            return Vec::new();
        }
        let canvas = self.canvas_rect;
        let mut objects = Vec::new();
        let toolbar = Rect::from_min_size(
            pos2(canvas.center().x - 108.0, canvas.bottom() - 56.0),
            vec2(216.0, 48.0),
        );
        Area::new(Id::new("pages-atelier-return"))
            .fixed_pos(toolbar.min)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                let choices: &[&str] = &["Arrange", "Paper"];
                for (index, &name) in choices.iter().enumerate() {
                    let cell = Rect::from_min_size(
                        toolbar.min + vec2(index as f32 * 108.0, 0.0),
                        vec2(108.0, 48.0),
                    );
                    let on = ctx.data(|d| {
                        d.get_temp::<bool>(Id::new("pages-material-open"))
                            .unwrap_or(false)
                    });
                    let selected = (name == "Paper" && on) || (name == "Arrange" && !on);
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
                    ctx.data_mut(|d| d.insert_temp(Id::new(("pages-atelier-tab", name)), cell));
                    if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        ctx.data_mut(|d| {
                            d.insert_temp(Id::new("pages-material-open"), name == "Paper")
                        });
                    }
                    objects.push(cell);
                }
            });
        objects
    }

    pub(super) fn paint_layout_instrument(&self, p: &Painter, rect: Rect) {
        // Same leather case language as the floating tool dock: notebook cover
        // pigment, soft shadow, and a quiet edge sheen.
        let radius = 22.0;
        let case = self.note_case_color();
        p.rect_filled(
            rect.translate(vec2(0.0, 3.0)),
            radius,
            self.look.shadow.gamma_multiply(0.55),
        );
        p.rect_filled(rect, radius, case);
        p.rect_stroke(
            rect.shrink(0.5),
            radius,
            Stroke::new(1.0, mix_col(case, self.look.paper, 0.22)),
            StrokeKind::Inside,
        );
        paint_case_sheen(p, rect, self.look.paper, self.look.shadow);
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
        let tone = self
            .note
            .as_ref()
            .map_or(PaperTone::Default, |note| note.paper_tone);
        let fill = self.look.paper_surface(tone);
        let ink = self.look.paper_ink(tone);
        let rule = mix_col(fill, ink, 0.48);
        let margin = self.look.paper_margin(tone);
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
            fill,
            Stroke::new(
                if hot || selected { 1.5 } else { 0.8 },
                if hot || selected {
                    self.look.accent
                } else {
                    mix_col(fill, ink, 0.55)
                },
            ),
        ));
        let line = |a: (f32, f32), b: (f32, f32), color: Color32| {
            p.line_segment([point(a.0, a.1), point(b.0, b.1)], Stroke::new(1.15, color));
        };
        match kind {
            PaperKind::Blank => {}
            PaperKind::Dots => {
                for x in 0..5 {
                    for y in 0..8 {
                        p.circle_filled(
                            point(-20.0 + x as f32 * 10.0, -32.0 + y as f32 * 9.0),
                            1.35,
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
                    line((-12.0, -37.0), (-12.0, 36.0), margin);
                    if kind == PaperKind::Cornell {
                        line((-23.0, 25.0), (23.0, 25.0), margin);
                    }
                }
            }
        }
        if selected {
            paint_check(p, point(0.0, 37.0), well_glyph(fill, self.look.paper, ink));
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
        let face = cell.shrink2(vec2(4.0, 6.0));
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
