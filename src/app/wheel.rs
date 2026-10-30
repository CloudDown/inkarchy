//! A little pigment case: separate wells around a paper proof, rather than pie slices.
use super::*;

pub(super) struct WheelPalette<'a> {
    p: &'a Painter,
    look: &'a Look,
    center: Pos2,
    scale: f32,
    case: Color32,
    paper: TextureId,
}

impl<'a> WheelPalette<'a> {
    pub(super) fn new(
        p: &'a Painter,
        center: Pos2,
        scale: f32,
        look: &'a Look,
        case: Color32,
        paper: TextureId,
    ) -> Self {
        Self {
            p,
            look,
            center,
            scale,
            case,
            paper,
        }
    }

    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.center + vec2(x, y) * self.scale
    }

    fn shadow(&self, at: Pos2, radius: f32, strength: f32) {
        for i in (0..6).rev() {
            self.p.circle_filled(
                at + vec2(0.0, 3.0 * self.scale),
                (radius + i as f32 * 1.2) * self.scale,
                self.look.shadow.gamma_multiply(strength * 0.12),
            );
        }
    }

    fn disc(&self, at: Pos2, radius: f32, top: Color32, bottom: Color32) {
        let mut mesh = Mesh::default();
        let radius = radius * self.scale;
        let feather = 0.5 / self.p.ctx().pixels_per_point();
        mesh.colored_vertex(at, mix_col(top, bottom, 0.5));
        for i in 0..=64 {
            let unit = Vec2::angled(i as f32 * std::f32::consts::TAU / 64.0);
            mesh.colored_vertex(
                at + unit * (radius - feather).max(0.0),
                mix_col(top, bottom, (unit.y + 1.0) * 0.5),
            );
            mesh.colored_vertex(at + unit * (radius + feather), Color32::TRANSPARENT);
            if i < 64 {
                let v = 1 + i * 2;
                mesh.add_triangle(0, v, v + 2);
                mesh.add_triangle(v, v + 1, v + 3);
                mesh.add_triangle(v, v + 3, v + 2);
            }
        }
        self.p.add(Shape::mesh(mesh));
    }

    fn grain(&self, at: Pos2, radius: f32) {
        // Local UVs stay inside the texture, including in CPU preview renders.
        let mut mesh = Mesh::with_texture(self.paper);
        let tint = Color32::WHITE.gamma_multiply(0.16);
        mesh.vertices.push(Vertex {
            pos: at,
            uv: pos2(0.5, 0.5),
            color: tint,
        });
        for i in 0..=64 {
            let unit = Vec2::angled(i as f32 * std::f32::consts::TAU / 64.0);
            mesh.vertices.push(Vertex {
                pos: at + unit * radius * self.scale,
                uv: pos2(0.5 + unit.x * 0.45, 0.5 + unit.y * 0.45),
                color: tint,
            });
            if i < 64 {
                mesh.add_triangle(0, i + 1, i + 2);
            }
        }
        self.p.add(Shape::mesh(mesh));
    }

    pub(super) fn paint(&self) {
        self.shadow(self.center, 150.0, 1.0);
        // A dark cloth shell, with one light catching its rolled edge.
        self.disc(
            self.at(0.0, 2.0),
            150.0,
            self.case,
            shade_rgb(self.case, 0.7),
        );
        self.disc(
            self.center,
            149.0,
            mix_col(self.case, self.look.paper, 0.09),
            self.case,
        );
        self.p.circle_stroke(
            self.center,
            148.5 * self.scale,
            Stroke::new(self.scale, self.look.paper.gamma_multiply(0.22)),
        );
        self.p.circle_stroke(
            self.center,
            142.0 * self.scale,
            Stroke::new(self.scale, self.look.ink.gamma_multiply(0.2)),
        );
        self.p.circle_stroke(
            self.at(0.0, 0.7),
            141.0 * self.scale,
            Stroke::new(self.scale, self.look.paper.gamma_multiply(0.07)),
        );

        // A warm paper proof in the middle, inset into the case.
        self.p.circle_filled(
            self.at(0.0, -0.5),
            59.0 * self.scale,
            shade_rgb(self.case, 0.72),
        );
        self.shadow(self.center, 54.0, 0.6);
        self.disc(
            self.center,
            54.0,
            self.look.paper,
            mix_col(self.look.paper, self.case, 0.045),
        );
        self.grain(self.center, 53.5);
        self.p.circle_stroke(
            self.center,
            49.0 * self.scale,
            Stroke::new(0.7 * self.scale, self.look.ink.gamma_multiply(0.15)),
        );
    }

    fn seat(&self, index: usize, count: usize, hot: bool) -> (Pos2, f32) {
        let orbit = 105.0;
        let radius = if count <= 2 {
            27.0
        } else {
            (orbit * (std::f32::consts::PI / count as f32).sin() - 5.0).clamp(1.0, 27.0)
        };
        (
            slice_mid(
                self.center,
                index,
                count,
                (orbit + if hot { 3.0 } else { 0.0 }) * self.scale,
            ),
            radius,
        )
    }

    fn selection(&self, at: Pos2, radius: f32, hot: bool, selected: bool) {
        if hot || selected {
            self.p.circle_stroke(
                at,
                (radius + 4.0) * self.scale,
                Stroke::new(
                    if hot { 1.7 } else { 1.2 } * self.scale,
                    self.look
                        .paper
                        .gamma_multiply(if hot { 0.95 } else { 0.55 }),
                ),
            );
        }
    }

    pub(super) fn color(
        &self,
        index: usize,
        count: usize,
        color: Color32,
        hot: bool,
        selected: bool,
    ) {
        let (at, radius) = self.seat(index, count, hot);
        self.p.circle_filled(
            at + vec2(0.0, -self.scale),
            (radius + 2.0) * self.scale,
            shade_rgb(self.case, 0.65),
        );
        self.shadow(at, radius, 0.65);
        self.disc(
            at,
            radius,
            mix_col(color, self.look.paper, 0.12),
            shade_rgb(color, 0.92),
        );
        self.p.circle_stroke(
            at,
            radius * self.scale,
            Stroke::new(self.scale, self.look.paper.gamma_multiply(0.22)),
        );
        self.selection(at, radius, hot, selected);
        if selected {
            let fg = well_glyph(color, self.look.paper, self.look.ink);
            let s = self.scale * (radius / 22.0).min(1.0);
            self.p.add(Shape::line(
                vec![
                    at + vec2(-4.0, 0.0) * s,
                    at + vec2(-1.0, 3.0) * s,
                    at + vec2(5.0, -4.0) * s,
                ],
                Stroke::new(1.8 * s, fg),
            ));
        }
    }

    // A continuous, round-ended ink gesture, rather than a thickness dot.
    fn ink_sample(&self, at: Pos2, half_width: f32, width: f32, ink: Color32) {
        let points: Vec<_> = (0..=24)
            .map(|i| {
                let t = i as f32 / 24.0;
                at + vec2(
                    (t * 2.0 - 1.0) * half_width,
                    -(t * std::f32::consts::TAU).sin() * 3.2,
                ) * self.scale
            })
            .collect();
        let thickness = width * self.scale;
        // The quiet keyline makes white ink visible on the paper proof.
        let outline = self.look.ink.gamma_multiply(0.2);
        for (w, color) in [(thickness + 1.0 * self.scale, outline), (thickness, ink)] {
            self.p
                .add(Shape::line(points.clone(), Stroke::new(w, color)));
            for end in [points[0], points[points.len() - 1]] {
                self.p.circle_filled(end, w * 0.5, color);
            }
        }
    }

    pub(super) fn nib(
        &self,
        index: usize,
        count: usize,
        width: f32,
        ink: Color32,
        hot: bool,
        selected: bool,
    ) {
        let (at, radius) = self.seat(index, count, hot);
        self.shadow(at, radius, 0.7);
        self.disc(
            at,
            radius,
            self.look.paper,
            mix_col(self.look.paper, self.case, 0.07),
        );
        self.selection(at, radius, hot, selected);
        let ends = [
            at + vec2(-10.0, -2.0) * self.scale,
            at + vec2(10.0, -8.0) * self.scale,
        ];
        for (w, color) in [
            (width + 1.0, self.look.ink.gamma_multiply(0.2)),
            (width, ink),
        ] {
            self.p
                .line_segment(ends, Stroke::new(w * self.scale, color));
            for end in ends {
                self.p.circle_filled(end, w * self.scale * 0.5, color);
            }
        }
        self.p.text(
            at + vec2(0.0, 16.0 * self.scale),
            Align2::CENTER_CENTER,
            format!("{width:.1}"),
            self.look.mono(9.0 * self.scale),
            self.look.ink.gamma_multiply(0.68),
        );
        if selected {
            self.p.circle_filled(
                at + vec2(0.0, (radius + 4.0) * self.scale),
                2.4 * self.scale,
                self.look.paper,
            );
        }
    }

    fn book(&self, at: Pos2, color: Color32, size: f32) {
        let s = self.scale * size;
        let cover = Rect::from_center_size(at, vec2(34.0, 44.0) * s);
        self.p.rect_filled(
            cover.translate(vec2(0.0, 3.0) * s).expand(1.0 * s),
            4.0 * s,
            self.look.shadow.gamma_multiply(0.6),
        );
        self.p.rect_filled(
            cover.translate(vec2(2.5, 1.5) * s),
            3.0 * s,
            self.look.paper,
        );
        self.p.rect_filled(cover, 3.0 * s, color);
        self.p.rect_stroke(
            cover.shrink(2.5 * s),
            2.0 * s,
            Stroke::new(0.8 * s, self.look.paper.gamma_multiply(0.22)),
            StrokeKind::Inside,
        );
        self.p.vline(
            cover.left() + 5.5 * s,
            cover.y_range().shrink(1.0 * s),
            Stroke::new(2.0 * s, self.look.ink.gamma_multiply(0.23)),
        );
        self.p.vline(
            cover.left() + 7.0 * s,
            cover.y_range().shrink(1.0 * s),
            Stroke::new(0.6 * s, self.look.paper.gamma_multiply(0.2)),
        );
        let label = Rect::from_center_size(at + vec2(2.5, -5.0) * s, vec2(16.0, 9.0) * s);
        self.p
            .rect_filled(label, 1.0 * s, self.look.paper.gamma_multiply(0.82));
        self.p.hline(
            label.x_range().shrink(3.0 * s),
            label.center().y,
            Stroke::new(0.8 * s, self.look.ink.gamma_multiply(0.3)),
        );
    }

    pub(super) fn cover(
        &self,
        index: usize,
        count: usize,
        color: Color32,
        hot: bool,
        selected: bool,
    ) {
        let (at, radius) = self.seat(index, count, hot);
        self.selection(at, radius, hot, selected);
        self.book(at, color, 1.0);
        if selected {
            self.p.circle_filled(
                at + vec2(0.0, 31.0 * self.scale),
                2.5 * self.scale,
                self.look.paper,
            );
        }
    }

    fn heading(&self, title: &str) {
        self.p.text(
            self.at(0.0, -25.0),
            Align2::CENTER_CENTER,
            title,
            self.look.serif(19.0 * self.scale),
            self.look.ink,
        );
    }

    fn caption(&self, label: &str) {
        self.p.text(
            self.at(0.0, 29.0),
            Align2::CENTER_CENTER,
            label,
            self.look.mono(9.0 * self.scale),
            self.look.ink.gamma_multiply(0.6),
        );
    }

    pub(super) fn ink_preview(&self, color: Color32) {
        self.heading("Ink");
        self.ink_sample(self.at(0.0, 3.0), 22.0, 7.0, color);
        self.caption(&format!(
            "#{:02X}{:02X}{:02X}",
            color.r(),
            color.g(),
            color.b()
        ));
    }

    pub(super) fn nib_preview(&self, width: f32) {
        self.heading("Nib");
        self.p.text(
            self.at(0.0, 3.0),
            Align2::CENTER_CENTER,
            format!("{width:.1}"),
            self.look.serif(29.0 * self.scale),
            self.look.ink,
        );
        self.caption("px · width");
    }

    pub(super) fn cover_preview(&self, color: Color32) {
        self.heading("Cover");
        self.book(self.at(0.0, 9.0), color, 0.85);
    }

    pub(super) fn action(&self, index: usize, fill: Color32, hot: bool, label: &str) -> Pos2 {
        let at = slice_mid(self.center, index, 3, 100.0 * self.scale);
        self.shadow(at, 27.0, 0.6);
        self.disc(at, 27.0, fill, mix_col(fill, self.case, 0.08));
        self.selection(at, 27.0, hot, false);
        self.p.text(
            at + vec2(0.0, 40.0 * self.scale),
            Align2::CENTER_CENTER,
            label,
            self.look.mono(10.0 * self.scale),
            self.look.paper,
        );
        at
    }

    pub(super) fn notebook_preview(&self, color: Color32) {
        self.p.text(
            self.at(0.0, -25.0),
            Align2::CENTER_CENTER,
            "Notebook",
            self.look.serif(15.0 * self.scale),
            self.look.ink,
        );
        self.book(self.at(0.0, 9.0), color, 0.85);
    }
}
