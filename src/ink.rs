//! Vector ink: points, pressure, ribbon, shapes.

use egui::{epaint::Vertex, Color32, Mesh, Pos2, TextureId, Vec2};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Nib {
    #[default]
    Fineliner,
    Brush,
    Pencil,
    Highlighter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Tool {
    #[default]
    Fineliner,
    Brush,
    Pencil,
    Highlighter,
    EraserStroke,
    EraserArea,
    Lasso,
    Text,
    Image,
}

impl Tool {
    pub fn nib(self) -> Option<Nib> {
        match self {
            Tool::Fineliner => Some(Nib::Fineliner),
            Tool::Brush => Some(Nib::Brush),
            Tool::Pencil => Some(Nib::Pencil),
            Tool::Highlighter => Some(Nib::Highlighter),
            _ => None,
        }
    }

    pub fn is_ink(self) -> bool {
        self.nib().is_some()
    }

    pub fn is_eraser(self) -> bool {
        matches!(self, Tool::EraserStroke | Tool::EraserArea)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Fineliner => "Pen",
            Tool::Brush => "Brush",
            Tool::Pencil => "Pencil",
            Tool::Highlighter => "Highlighter",
            Tool::EraserStroke => "stroke",
            Tool::EraserArea => "area",
            Tool::Lasso => "Lasso",
            Tool::Text => "Text",
            Tool::Image => "Image",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct InkPoint {
    pub x: f32,
    pub y: f32,
    pub p: f32,
}

impl InkPoint {
    pub fn new(pos: Pos2, p: f32) -> Self {
        Self {
            x: pos.x,
            y: pos.y,
            p: p.clamp(0.08, 1.0),
        }
    }

    pub fn pos(self) -> Pos2 {
        Pos2::new(self.x, self.y)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InkStroke {
    pub id: Uuid,
    pub nib: Nib,
    pub color: [u8; 4],
    pub width: f32,
    pub points: Vec<InkPoint>,
    #[serde(skip)]
    pub mesh: Option<Mesh>,
}

impl InkStroke {
    pub fn new(nib: Nib, color: Color32, width: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            nib,
            color: color.to_array(),
            width,
            points: Vec::with_capacity(64),
            mesh: None,
        }
    }

    /// Snap a mostly-horizontal highlighter onto one ruled line.
    pub fn cling_to_line(&mut self, y: f32) {
        if self.nib != Nib::Highlighter || self.points.len() < 2 {
            return;
        }
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for p in &self.points {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }
        if max_x - min_x < 18.0 || (max_x - min_x) < (max_y - min_y) * 1.2 {
            return;
        }
        self.points = vec![
            InkPoint::new(Pos2::new(min_x, y), 1.0),
            InkPoint::new(Pos2::new(max_x, y), 1.0),
        ];
        self.mesh = None;
    }

    pub fn color32(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.color[0], self.color[1], self.color[2], self.color[3])
    }

    pub fn push(&mut self, p: InkPoint) {
        if let Some(last) = self.points.last() {
            let dx = p.x - last.x;
            let dy = p.y - last.y;
            if dx * dx + dy * dy < 0.36 {
                return;
            }
        }
        self.points.push(p);
        self.mesh = None;
    }

    pub fn translate(&mut self, d: Vec2) {
        for p in &mut self.points {
            p.x += d.x;
            p.y += d.y;
        }
        self.mesh = None;
    }

    pub fn bbox(&self) -> Option<(Pos2, Pos2)> {
        let mut it = self.points.iter();
        let first = it.next()?;
        let mut min = first.pos();
        let mut max = min;
        for p in it {
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
        }
        Some((min, max))
    }

    pub fn hits(&self, pos: Pos2, extra: f32) -> bool {
        let r = self.width * 0.55 + extra;
        let Some((min, max)) = self.bbox() else {
            return false;
        };
        if pos.x + r < min.x || pos.y + r < min.y || pos.x - r > max.x || pos.y - r > max.y {
            return false;
        }
        let r2 = r * r;
        for w in self.points.windows(2) {
            if dist2_seg(pos, w[0].pos(), w[1].pos()) <= r2 {
                return true;
            }
        }
        if self.points.len() == 1 {
            return self.points[0].pos().distance_sq(pos) <= r2;
        }
        false
    }

    pub fn tessellate(&mut self) -> &Mesh {
        if self.mesh.is_none() {
            let mesh = ribbon_mesh(&self.points, self.width, self.nib, self.color32());
            self.mesh = Some(mesh);
        }
        self.mesh.as_ref().expect("ribbon mesh was stored above")
    }
}

impl Clone for InkStroke {
    /// Points travel with the stroke. The ribbon mesh is a cache and is rebuilt on paint.
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            nib: self.nib,
            color: self.color,
            width: self.width,
            points: self.points.clone(),
            mesh: None,
        }
    }
}

/// Paper-space ribbon into screen space: `x' = ox + x * sx`, `y' = oy + y * sy`.
pub fn map_mesh(src: &Mesh, ox: f32, oy: f32, sx: f32, sy: f32) -> Mesh {
    let mut mesh = src.clone();
    for v in &mut mesh.vertices {
        v.pos.x = ox + v.pos.x * sx;
        v.pos.y = oy + v.pos.y * sy;
    }
    mesh
}

pub fn ribbon_mesh(points: &[InkPoint], width: f32, nib: Nib, color: Color32) -> Mesh {
    let mut mesh = Mesh::with_texture(TextureId::default());
    if points.is_empty() {
        return mesh;
    }
    let color = premultiply(color);
    if points.len() == 1 {
        add_disc(
            &mut mesh,
            points[0].pos(),
            width * 0.45 * points[0].p,
            color,
        );
        return mesh;
    }

    let n = points.len();
    // Two end discs (13 rim verts) plus a shared-vertex strip: 2 verts per sample.
    mesh.vertices.reserve(2 * 14 + 2 * n);
    mesh.indices.reserve(2 * 36 + (n - 1) * 6);

    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for (i, pt) in points.iter().enumerate() {
        let tan = if i == 0 {
            points[1].pos() - points[0].pos()
        } else if i + 1 == points.len() {
            points[i].pos() - points[i - 1].pos()
        } else {
            points[i + 1].pos() - points[i - 1].pos()
        };
        let n = rot90(tan.normalized());
        let mut press = pt.p;
        match nib {
            Nib::Fineliner => press = 1.0,
            Nib::Highlighter => press = 1.0,
            Nib::Pencil => press = 0.35 + press * 0.65,
            Nib::Brush => press = 0.18 + press * 0.82,
        }
        let mut r = width * 0.5 * press;
        if nib == Nib::Highlighter {
            r = width * 0.5;
        }
        r = r.max(0.25);
        left.push(pt.pos() + n * r);
        right.push(pt.pos() - n * r);
    }

    let Some(first) = points.first() else {
        return mesh;
    };
    let Some(last) = points.last() else {
        return mesh;
    };
    let Some(left_first) = left.first() else {
        return mesh;
    };
    let Some(left_last) = left.last() else {
        return mesh;
    };
    add_disc(
        &mut mesh,
        first.pos(),
        dist(first.pos(), *left_first),
        color,
    );
    add_disc(&mut mesh, last.pos(), dist(last.pos(), *left_last), color);

    let body = mesh.vertices.len() as u32;
    for i in 0..n {
        mesh.vertices.push(Vertex {
            pos: left[i],
            uv: Pos2::ZERO,
            color,
        });
        mesh.vertices.push(Vertex {
            pos: right[i],
            uv: Pos2::ZERO,
            color,
        });
    }
    for i in 0..n - 1 {
        let l0 = body + (i as u32) * 2;
        let r0 = l0 + 1;
        let l1 = l0 + 2;
        let r1 = l0 + 3;
        mesh.indices.extend_from_slice(&[l0, r0, l1, r0, r1, l1]);
    }
    mesh
}

pub fn ribbon_outline(points: &[InkPoint], width: f32, nib: Nib) -> Vec<Pos2> {
    if points.is_empty() {
        return vec![];
    }
    if points.len() == 1 {
        return circle_pts(points[0].pos(), width * 0.45 * points[0].p, 14);
    }
    let mut left = Vec::new();
    let mut right = Vec::new();
    for (i, pt) in points.iter().enumerate() {
        let tan = if i == 0 {
            points[1].pos() - points[0].pos()
        } else if i + 1 == points.len() {
            points[i].pos() - points[i - 1].pos()
        } else {
            points[i + 1].pos() - points[i - 1].pos()
        };
        let n = rot90(tan.normalized());
        let mut press = pt.p;
        match nib {
            Nib::Fineliner | Nib::Highlighter => press = 1.0,
            Nib::Pencil => press = 0.35 + press * 0.65,
            Nib::Brush => press = 0.18 + press * 0.82,
        }
        let r = (width * 0.5 * press).max(0.25);
        left.push(pt.pos() + n * r);
        right.push(pt.pos() - n * r);
    }
    left.extend(right.into_iter().rev());
    left
}

fn add_disc(mesh: &mut Mesh, c: Pos2, r: f32, color: Color32) {
    let r = r.max(0.3);
    let n = 12u32;
    let start = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        pos: c,
        uv: Pos2::ZERO,
        color,
    });
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        mesh.vertices.push(Vertex {
            pos: c + Vec2::angled(a) * r,
            uv: Pos2::ZERO,
            color,
        });
    }
    for i in 0..n {
        mesh.indices
            .extend_from_slice(&[start, start + i + 1, start + i + 2]);
    }
}

fn circle_pts(c: Pos2, r: f32, n: usize) -> Vec<Pos2> {
    (0..n)
        .map(|i| c + Vec2::angled(i as f32 / n as f32 * std::f32::consts::TAU) * r)
        .collect()
}

fn rot90(v: Vec2) -> Vec2 {
    if v.length_sq() < 1e-8 {
        Vec2::new(0.0, -1.0)
    } else {
        Vec2::new(-v.y, v.x)
    }
}

fn dist(a: Pos2, b: Pos2) -> f32 {
    a.distance(b)
}

fn dist2_seg(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-8)).clamp(0.0, 1.0);
    (a + ab * t).distance_sq(p)
}

pub fn point_in_poly(p: Pos2, poly: &[Pos2]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let pi = poly[i];
        let pj = poly[j];
        if ((pi.y > p.y) != (pj.y > p.y))
            && (p.x < (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y + 1e-8) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn premultiply(c: Color32) -> Color32 {
    let a = c.a() as u16;
    Color32::from_rgba_premultiplied(
        (c.r() as u16 * a / 255) as u8,
        (c.g() as u16 * a / 255) as u8,
        (c.b() as u16 * a / 255) as u8,
        c.a(),
    )
}

/// Hardware pressure, otherwise speed (fountain pen: slow means fatter).
pub fn mixed_pressure(nib: Nib, hw: Option<f32>, speed: f32) -> f32 {
    if let Some(p) = hw {
        return p.clamp(0.08, 1.0);
    }
    match nib {
        Nib::Fineliner | Nib::Highlighter => 1.0,
        Nib::Brush => {
            let t = (speed / 900.0).clamp(0.0, 1.0);
            0.95 - t * 0.55
        }
        Nib::Pencil => {
            let t = (speed / 700.0).clamp(0.0, 1.0);
            0.55 + (1.0 - t) * 0.35
        }
    }
}

pub fn maybe_snap_shape(stroke: &InkStroke) -> Option<InkStroke> {
    if stroke.points.len() < 6 {
        return None;
    }
    if stroke_closed(stroke) {
        return best_closed_shape(stroke);
    }
    if let Some(s) = fit_arrow(stroke) {
        return Some(s);
    }
    if line_fits(stroke) {
        return Some(line_stroke(stroke));
    }
    None
}

/// Picks the closest closed shape (the best fit, not the first that passes).
fn best_closed_shape(stroke: &InkStroke) -> Option<InkStroke> {
    let mut best: Option<(f32, InkStroke)> = None;
    let mut consider = |score: f32, s: Option<InkStroke>| {
        let Some(s) = s else {
            return;
        };
        if best.as_ref().map(|(e, _)| score < *e).unwrap_or(true) {
            best = Some((score, s));
        }
    };
    if let Some((score, s)) = fit_triangle_scored(stroke) {
        consider(score, Some(s));
    }
    if let Some((score, s)) = fit_diamond_scored(stroke) {
        consider(score, Some(s));
    }
    if let Some((score, s)) = fit_rect_scored(stroke) {
        consider(score, Some(s));
    }
    if let Some((score, s)) = fit_ellipse_scored(stroke) {
        consider(score, Some(s));
    }
    best.map(|(_, s)| s)
}

fn stroke_closed(stroke: &InkStroke) -> bool {
    let Some(start) = stroke.points.first().map(|p| p.pos()) else {
        return false;
    };
    let Some(end) = stroke.points.last().map(|p| p.pos()) else {
        return false;
    };
    let gap = start.distance(end);
    let path: f32 = stroke
        .points
        .windows(2)
        .map(|w| w[0].pos().distance(w[1].pos()))
        .sum();
    if path < 40.0 {
        return false;
    }
    let diag = stroke.bbox().map(|(a, b)| a.distance(b)).unwrap_or(path);
    // Approximate closure: a real gap at the end of the stroke is allowed.
    gap < (path * 0.38).min(diag * 0.42).max(56.0) || (gap < 90.0 && path > 70.0)
}

fn line_fits(stroke: &InkStroke) -> bool {
    let Some(a) = stroke.points.first().map(|p| p.pos()) else {
        return false;
    };
    let Some(b) = stroke.points.last().map(|p| p.pos()) else {
        return false;
    };
    let len = a.distance(b);
    if len < 28.0 {
        return false;
    }
    let allow = (len * 0.07).clamp(8.0, 22.0);
    line_error(stroke) < allow
}

fn line_stroke(stroke: &InkStroke) -> InkStroke {
    let (Some(&a), Some(&b)) = (stroke.points.first(), stroke.points.last()) else {
        let mut empty = stroke.clone();
        empty.mesh = None;
        return empty;
    };
    let mut s = stroke.clone();
    s.id = Uuid::new_v4();
    s.points = vec![
        a,
        InkPoint {
            x: b.x,
            y: b.y,
            p: a.p,
        },
    ];
    s.mesh = None;
    s
}

fn line_error(stroke: &InkStroke) -> f32 {
    let Some(a) = stroke.points.first().map(|p| p.pos()) else {
        return f32::MAX;
    };
    let Some(b) = stroke.points.last().map(|p| p.pos()) else {
        return f32::MAX;
    };
    let mut e = 0.0f32;
    for p in &stroke.points {
        e = e.max(dist2_seg(p.pos(), a, b).sqrt());
    }
    e
}

fn poly_score(stroke: &InkStroke, verts: &[Pos2]) -> f32 {
    if verts.len() < 2 || stroke.points.is_empty() {
        return f32::MAX;
    }
    let mut sum = 0.0f32;
    let mut worst = 0.0f32;
    for p in &stroke.points {
        let mut d = f32::MAX;
        for i in 0..verts.len() {
            let a = verts[i];
            let b = verts[(i + 1) % verts.len()];
            d = d.min(dist2_seg(p.pos(), a, b).sqrt());
        }
        sum += d;
        worst = worst.max(d);
    }
    let mean = sum / stroke.points.len() as f32;
    mean * 0.7 + worst * 0.3
}

fn stroke_poly(proto: &InkStroke, verts: &[Pos2], closed: bool, steps: usize) -> InkStroke {
    let mut pts = Vec::new();
    let n = if closed {
        verts.len()
    } else {
        verts.len().saturating_sub(1)
    };
    for i in 0..n {
        let a = verts[i];
        let b = verts[(i + 1) % verts.len()];
        for k in 0..=steps {
            if k == 0 && i > 0 {
                continue;
            }
            let t = k as f32 / steps as f32;
            pts.push(InkPoint::new(a.lerp(b, t), 1.0));
        }
    }
    let mut s = proto.clone();
    s.id = Uuid::new_v4();
    s.points = pts;
    s.mesh = None;
    s
}

fn fit_ellipse_scored(stroke: &InkStroke) -> Option<(f32, InkStroke)> {
    let (min, max) = stroke.bbox()?;
    let w = max.x - min.x;
    let h = max.y - min.y;
    if w < 24.0 || h < 24.0 {
        return None;
    }
    // Pulled in a little: a freehand stroke often spills past the true oval.
    let pad = 0.04;
    let rx = w * 0.5 * (1.0 - pad);
    let ry = h * 0.5 * (1.0 - pad);
    let c = Pos2::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5);
    let mut sum = 0.0f32;
    let mut worst = 0.0f32;
    for p in &stroke.points {
        let dx = (p.x - c.x) / rx.max(1.0);
        let dy = (p.y - c.y) / ry.max(1.0);
        let e = ((dx * dx + dy * dy).sqrt() - 1.0).abs();
        sum += e;
        worst = worst.max(e);
    }
    let mean = sum / stroke.points.len() as f32;
    // Very tolerant: "almost an oval" should pass.
    if mean > 0.28 || worst > 0.55 {
        return None;
    }
    let aspect = (rx / ry).max(ry / rx);
    let pts: Vec<InkPoint> = if aspect < 1.12 {
        let r = (rx + ry) * 0.5;
        circle_pts(c, r, 48)
            .into_iter()
            .map(|p| InkPoint::new(p, 1.0))
            .collect()
    } else {
        (0..56)
            .map(|i| {
                let a = i as f32 / 56.0 * std::f32::consts::TAU;
                InkPoint::new(Pos2::new(c.x + a.cos() * rx, c.y + a.sin() * ry), 1.0)
            })
            .collect()
    };
    let mut s = stroke.clone();
    s.id = Uuid::new_v4();
    s.points = pts;
    s.mesh = None;
    // Score normalized by size so it can be compared with polygons.
    let scale = rx.min(ry).max(1.0);
    Some(((mean * 0.65 + worst * 0.35) * scale, s))
}

fn fit_rect_scored(stroke: &InkStroke) -> Option<(f32, InkStroke)> {
    let (min, max) = stroke.bbox()?;
    let w = max.x - min.x;
    let h = max.y - min.y;
    if w < 18.0 || h < 18.0 {
        return None;
    }
    let corners = [
        Pos2::new(min.x, min.y),
        Pos2::new(max.x, min.y),
        Pos2::new(max.x, max.y),
        Pos2::new(min.x, max.y),
    ];
    let score = poly_score(stroke, &corners);
    let allow = (w.min(h) * 0.18).clamp(16.0, 40.0);
    if score > allow {
        return None;
    }
    Some((score, stroke_poly(stroke, &corners, true, 8)))
}

fn fit_diamond_scored(stroke: &InkStroke) -> Option<(f32, InkStroke)> {
    let (min, max) = stroke.bbox()?;
    let w = max.x - min.x;
    let h = max.y - min.y;
    if w < 24.0 || h < 24.0 {
        return None;
    }
    let cx = (min.x + max.x) * 0.5;
    let cy = (min.y + max.y) * 0.5;
    let verts = [
        Pos2::new(cx, min.y),
        Pos2::new(max.x, cy),
        Pos2::new(cx, max.y),
        Pos2::new(min.x, cy),
    ];
    let score = poly_score(stroke, &verts);
    let allow = (w.min(h) * 0.18).clamp(16.0, 42.0);
    if score > allow {
        return None;
    }
    let rect_corners = [
        Pos2::new(min.x, min.y),
        Pos2::new(max.x, min.y),
        Pos2::new(max.x, max.y),
        Pos2::new(min.x, max.y),
    ];
    let corner_gap = rect_corners
        .iter()
        .map(|c| {
            stroke
                .points
                .iter()
                .map(|p| p.pos().distance(*c))
                .fold(f32::MAX, f32::min)
        })
        .fold(0.0f32, f32::min);
    if corner_gap < (w.min(h) * 0.08).clamp(6.0, 22.0) {
        return None;
    }
    Some((score, stroke_poly(stroke, &verts, true, 8)))
}

fn fit_triangle_scored(stroke: &InkStroke) -> Option<(f32, InkStroke)> {
    let (min, max) = stroke.bbox()?;
    let w = max.x - min.x;
    let h = max.y - min.y;
    if w < 28.0 || h < 28.0 {
        return None;
    }
    let verts = triangle_verts(stroke)?;
    let score = poly_score(stroke, &verts);
    let allow = (w.min(h) * 0.22).clamp(18.0, 48.0);
    if score > allow {
        return None;
    }
    Some((score, stroke_poly(stroke, &verts, true, 10)))
}

fn triangle_verts(stroke: &InkStroke) -> Option<[Pos2; 3]> {
    // 1) Corners from a sharp turn
    if let Some(v) = triangle_from_turns(stroke) {
        return Some(v);
    }
    // 2) Fallback: largest area among samples
    let n = stroke.points.len();
    let step = (n / 32).max(1);
    let samples: Vec<Pos2> = stroke
        .points
        .iter()
        .step_by(step)
        .map(|p| p.pos())
        .collect();
    if samples.len() < 3 {
        return None;
    }
    let mut best = None;
    let mut best_area = 0.0f32;
    for i in 0..samples.len() {
        for j in (i + 1)..samples.len() {
            for k in (j + 1)..samples.len() {
                let a = samples[i];
                let b = samples[j];
                let c = samples[k];
                let area = ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5;
                if area > best_area {
                    best_area = area;
                    best = Some([a, b, c]);
                }
            }
        }
    }
    let verts = best?;
    let (min, max) = stroke.bbox()?;
    let box_area = (max.x - min.x).max(1.0) * (max.y - min.y).max(1.0);
    if best_area < box_area * 0.16 {
        return None;
    }
    Some(order_triangle(verts))
}

fn triangle_from_turns(stroke: &InkStroke) -> Option<[Pos2; 3]> {
    let pts: Vec<Pos2> = stroke.points.iter().map(|p| p.pos()).collect();
    if pts.len() < 9 {
        return None;
    }
    let mut corners: Vec<(f32, Pos2)> = Vec::new();
    let win = (pts.len() / 18).clamp(2, 8);
    for i in win..(pts.len() - win) {
        let a = pts[i - win];
        let b = pts[i];
        let c = pts[i + win];
        let v1 = (a - b).normalized();
        let v2 = (c - b).normalized();
        let cross = v1.x * v2.y - v1.y * v2.x;
        let dot = (v1.x * v2.x + v1.y * v2.y).clamp(-1.0, 1.0);
        let turn = cross.atan2(dot).abs();
        if turn > 0.55 {
            corners.push((turn, b));
        }
    }
    // Add start / end if the stroke is almost closed.
    corners.push((1.2, pts[0]));
    if let Some(last) = pts.last() {
        if last.distance(pts[0]) > 8.0 {
            corners.push((1.0, *last));
        }
    }
    corners.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut picked: Vec<Pos2> = Vec::new();
    let (min, max) = stroke.bbox()?;
    let min_sep = (min.distance(max) * 0.18).max(24.0);
    for (_, p) in corners {
        if picked.iter().all(|q| q.distance(p) >= min_sep) {
            picked.push(p);
        }
        if picked.len() == 3 {
            break;
        }
    }
    if picked.len() < 3 {
        return None;
    }
    let verts = [picked[0], picked[1], picked[2]];
    let area = ((verts[1].x - verts[0].x) * (verts[2].y - verts[0].y)
        - (verts[1].y - verts[0].y) * (verts[2].x - verts[0].x))
        .abs()
        * 0.5;
    let box_area = (max.x - min.x).max(1.0) * (max.y - min.y).max(1.0);
    if area < box_area * 0.14 {
        return None;
    }
    Some(order_triangle(verts))
}

fn order_triangle(verts: [Pos2; 3]) -> [Pos2; 3] {
    let c = Pos2::new(
        (verts[0].x + verts[1].x + verts[2].x) / 3.0,
        (verts[0].y + verts[1].y + verts[2].y) / 3.0,
    );
    let mut ordered = verts;
    ordered.sort_by(|a, b| {
        let aa = (a.y - c.y).atan2(a.x - c.x);
        let bb = (b.y - c.y).atan2(b.x - c.x);
        aa.partial_cmp(&bb).unwrap_or(std::cmp::Ordering::Equal)
    });
    ordered
}

fn fit_arrow(stroke: &InkStroke) -> Option<InkStroke> {
    let n = stroke.points.len();
    if n < 12 {
        return None;
    }
    let start = stroke.points[0].pos();
    let tip = stroke.points.iter().map(|p| p.pos()).max_by(|a, b| {
        a.distance(start)
            .partial_cmp(&b.distance(start))
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    let shaft = tip - start;
    let len = shaft.length();
    if len < 56.0 {
        return None;
    }
    let dir = shaft / len;
    let nrm = rot90(dir);
    let shaft_end = (n as f32 * 0.68) as usize;
    let mut shaft_err = 0.0f32;
    for p in &stroke.points[..shaft_end.max(2)] {
        shaft_err = shaft_err.max(dist2_seg(p.pos(), start, tip).sqrt());
    }
    if shaft_err > (len * 0.08).clamp(10.0, 24.0) {
        return None;
    }
    let mut left = 0.0f32;
    let mut right = 0.0f32;
    let head_from = ((n as f32 * 0.55) as usize).min(n - 2);
    for p in &stroke.points[head_from..] {
        let along = (p.pos() - start).dot(dir);
        if along < len * 0.55 {
            continue;
        }
        let side = (p.pos() - start).dot(nrm);
        if side > left {
            left = side;
        }
        if -side > right {
            right = -side;
        }
    }
    let head_w = left.min(right);
    if head_w < (len * 0.06).clamp(8.0, 32.0) {
        return None;
    }
    if left.max(right) > head_w * 3.2 {
        return None;
    }
    let base = tip - dir * (head_w * 1.55).clamp(18.0, len * 0.35);
    let wing = head_w * 1.05;
    let mut pts = Vec::new();
    for i in 0..8 {
        let t = i as f32 / 8.0;
        pts.push(InkPoint::new(start.lerp(tip, t), 1.0));
    }
    for i in 1..=6 {
        let t = i as f32 / 6.0;
        pts.push(InkPoint::new(tip.lerp(base + nrm * wing, t), 1.0));
    }
    pts.push(InkPoint::new(tip, 1.0));
    for i in 1..=6 {
        let t = i as f32 / 6.0;
        pts.push(InkPoint::new(tip.lerp(base - nrm * wing, t), 1.0));
    }
    let mut s = stroke.clone();
    s.id = Uuid::new_v4();
    s.points = pts;
    s.mesh = None;
    Some(s)
}

pub fn erase_area(stroke: &InkStroke, center: Pos2, radius: f32) -> Vec<InkStroke> {
    let r2 = radius * radius;
    let mut fragments = Vec::new();
    let mut cur: Vec<InkPoint> = Vec::new();
    let flush = |cur: &mut Vec<InkPoint>, fragments: &mut Vec<InkStroke>, proto: &InkStroke| {
        if cur.len() >= 2 {
            let mut s = proto.clone();
            s.id = Uuid::new_v4();
            s.points = std::mem::take(cur);
            s.mesh = None;
            fragments.push(s);
        } else {
            cur.clear();
        }
    };
    for p in &stroke.points {
        if p.pos().distance_sq(center) > r2 {
            cur.push(*p);
        } else {
            flush(&mut cur, &mut fragments, stroke);
        }
    }
    flush(&mut cur, &mut fragments, stroke);
    fragments
}

pub fn default_width(nib: Nib) -> f32 {
    match nib {
        Nib::Fineliner => 2.2,
        Nib::Brush => 6.5,
        Nib::Pencil => 3.4,
        Nib::Highlighter => 16.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn empty_stroke_does_not_panic() {
        let mesh = ribbon_mesh(&[], 2.2, Nib::Fineliner, Color32::BLACK);
        assert!(mesh.vertices.is_empty());

        let mut stroke = InkStroke::new(Nib::Brush, Color32::BLACK, 4.0);
        assert!(stroke.points.is_empty());
        assert!(maybe_snap_shape(&stroke).is_none());
        assert!(!stroke_closed(&stroke));
        assert!(!line_fits(&stroke));
        assert!(line_error(&stroke).is_infinite() || line_error(&stroke) == f32::MAX);
        assert!(line_stroke(&stroke).points.is_empty());
        assert!(stroke.tessellate().vertices.is_empty());

        stroke.push(InkPoint::new(pos2(1.0, 1.0), 0.8));
        assert!(maybe_snap_shape(&stroke).is_none());
        let _ = stroke.tessellate();
        assert!(!stroke_closed(&stroke));
        assert!(!line_fits(&stroke));
    }

    #[test]
    fn two_point_ribbon_has_geometry() {
        let points = [
            InkPoint::new(pos2(0.0, 0.0), 1.0),
            InkPoint::new(pos2(40.0, 0.0), 1.0),
        ];
        let mesh = ribbon_mesh(&points, 3.0, Nib::Fineliner, Color32::BLACK);
        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.indices.is_empty());
        assert!(mesh.is_valid());
        // Two discs plus one shared quad, not six fresh vertices per triangle.
        assert_eq!(mesh.vertices.len(), 2 * 14 + 4);
        assert_eq!(mesh.indices.len(), 2 * 36 + 6);
        let left = mesh.vertices.iter().any(|v| (v.pos.y - 1.5).abs() < 0.01);
        let right = mesh.vertices.iter().any(|v| (v.pos.y + 1.5).abs() < 0.01);
        assert!(left && right);
    }

    #[test]
    fn clone_drops_the_ribbon_cache() {
        let mut stroke = InkStroke::new(Nib::Fineliner, Color32::BLACK, 2.0);
        stroke.push(InkPoint::new(pos2(0.0, 0.0), 1.0));
        stroke.push(InkPoint::new(pos2(30.0, 4.0), 1.0));
        let _ = stroke.tessellate();
        assert!(stroke.mesh.is_some());
        let copy = stroke.clone();
        assert!(copy.mesh.is_none());
        assert_eq!(copy.points.len(), stroke.points.len());
        assert!(stroke.mesh.is_some());
    }

    #[test]
    fn map_mesh_is_an_affine_scale_and_shift() {
        let points = [
            InkPoint::new(pos2(0.0, 0.0), 1.0),
            InkPoint::new(pos2(10.0, 0.0), 1.0),
        ];
        let src = ribbon_mesh(&points, 2.0, Nib::Fineliner, Color32::BLACK);
        let mapped = map_mesh(&src, 5.0, -3.0, 2.0, 4.0);
        assert_eq!(mapped.indices, src.indices);
        assert_eq!(mapped.vertices.len(), src.vertices.len());
        for (before, after) in src.vertices.iter().zip(&mapped.vertices) {
            assert!((after.pos.x - (5.0 + before.pos.x * 2.0)).abs() < 1e-4);
            assert!((after.pos.y - (-3.0 + before.pos.y * 4.0)).abs() < 1e-4);
            assert_eq!(after.color, before.color);
        }
    }

    #[test]
    fn hits_rejects_points_outside_the_stroke_box() {
        let mut stroke = InkStroke::new(Nib::Fineliner, Color32::BLACK, 2.0);
        stroke.push(InkPoint::new(pos2(0.0, 0.0), 1.0));
        stroke.push(InkPoint::new(pos2(40.0, 0.0), 1.0));
        assert!(stroke.hits(pos2(20.0, 0.5), 1.0));
        assert!(!stroke.hits(pos2(20.0, 80.0), 1.0));
    }

    #[test]
    fn highlighter_clings_to_a_rule() {
        let mut s = InkStroke::new(Nib::Highlighter, Color32::YELLOW, 8.0);
        s.push(InkPoint::new(pos2(40.0, 90.0), 1.0));
        s.push(InkPoint::new(pos2(200.0, 98.0), 1.0));
        s.cling_to_line(88.0);
        assert_eq!(s.points.len(), 2);
        assert!((s.points[0].y - 88.0).abs() < 0.01);
        assert!((s.points[1].y - 88.0).abs() < 0.01);
        assert!(s.points[1].x > s.points[0].x);
    }
}
