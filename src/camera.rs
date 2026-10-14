use egui::{Pos2, Rect, Vec2};

pub const ZOOM_MIN: f32 = 0.05;
pub const ZOOM_MAX: f32 = 8.0;
/// Levels relative to the screen size (1.0 = the sheet fills the window).
pub const ZOOM_STOPS: [f32; 9] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0];

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pan: Vec2,
    pub zoom: f32,
    /// Vertical scale when X and Y differ. `None` keeps Y on `zoom`.
    /// Fit leaves this empty: fullscreen only changes `zoom`.
    pub stretch_y: Option<f32>,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            stretch_y: None,
        }
    }
}

impl Camera {
    pub fn valid_viewport(rect: Rect) -> bool {
        rect.is_finite() && rect.width() > 10.0 && rect.height() > 10.0
    }

    /// Reframe a fitted page, or retain the document point at the viewport center.
    pub fn resize_viewport(&mut self, previous: Rect, next: Rect, fitted: Option<(Vec2, Vec2)>) {
        if !Self::valid_viewport(next) {
            return;
        }
        if let Some((origin, size)) = fitted {
            self.fit_page(next, origin, size.x, size.y);
        } else if Self::valid_viewport(previous) {
            let focus = self.to_paper(previous.center(), previous);
            self.pan = Vec2::new(
                next.width() * 0.5 - focus.x * self.zoom,
                next.height() * 0.5 - focus.y * self.zoom_y(),
            );
        }
    }

    pub fn zoom_y(self) -> f32 {
        self.stretch_y.unwrap_or(self.zoom)
    }

    pub fn zoom_min(self) -> f32 {
        self.zoom.min(self.zoom_y())
    }

    /// Paper-to-screen scale. X and Y differ only when `stretch_y` is set.
    pub fn scale(self) -> Vec2 {
        Vec2::new(self.zoom, self.zoom_y())
    }

    pub fn to_screen(self, paper: Pos2, rect: Rect) -> Pos2 {
        Pos2::new(
            rect.min.x + self.pan.x + paper.x * self.zoom,
            rect.min.y + self.pan.y + paper.y * self.zoom_y(),
        )
    }

    pub fn to_paper(self, screen: Pos2, rect: Rect) -> Pos2 {
        Pos2::new(
            (screen.x - rect.min.x - self.pan.x) / self.zoom,
            (screen.y - rect.min.y - self.pan.y) / self.zoom_y(),
        )
    }

    pub fn screen_to_paper_delta(self, delta: Vec2) -> Vec2 {
        Vec2::new(delta.x / self.zoom, delta.y / self.zoom_y())
    }

    pub fn zoom_at(&mut self, screen: Pos2, rect: Rect, factor: f32) {
        let paper = self.to_paper(screen, rect);
        let zy = self.zoom_y();
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        let next_y = (zy * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        self.stretch_y = if (self.zoom - next_y).abs() < 0.0001 {
            None
        } else {
            Some(next_y)
        };
        let now = self.to_screen(paper, rect);
        self.pan += screen - now;
    }

    pub fn set_zoom_at(&mut self, screen: Pos2, rect: Rect, zoom: f32) {
        let target = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let factor = target / self.zoom.max(0.001);
        self.zoom_at(screen, rect, factor);
    }

    pub fn fit_zoom(rect: Rect, page_w: f32, page_h: f32) -> f32 {
        let margin = 8.0;
        ((rect.width() - margin * 2.0) / page_w.max(1.0))
            .min((rect.height() - margin * 2.0) / page_h.max(1.0))
            .clamp(ZOOM_MIN, ZOOM_MAX)
    }

    /// Frame one page at a single zoom. The sheet keeps its aspect and covers
    /// the viewport: the loose axis runs off-screen, centered.
    pub fn fit_page(&mut self, rect: Rect, origin: Vec2, page_w: f32, page_h: f32) {
        let page_w = page_w.max(1.0);
        let page_h = page_h.max(1.0);
        let zx = (rect.width() / page_w).clamp(ZOOM_MIN, ZOOM_MAX);
        let zy = (rect.height() / page_h).clamp(ZOOM_MIN, ZOOM_MAX);
        let z = zx.max(zy);
        self.zoom = z;
        self.stretch_y = None;
        self.pan = Vec2::new(
            (rect.width() - page_w * z) * 0.5 - origin.x * z,
            (rect.height() - page_h * z) * 0.5 - origin.y * z,
        );
    }

    /// Top-left corner: write into it, zoomed out enough to see the whole sheet.
    pub fn show_writing(&mut self, rect: Rect, origin: Vec2, page_w: f32, page_h: f32) {
        let margin = 8.0;
        let z_w = (rect.width() - margin * 2.0) / page_w.max(1.0);
        let z_fit = Self::fit_zoom(rect, page_w, page_h);
        self.zoom = z_w.max(z_fit * 1.7).clamp(ZOOM_MIN, ZOOM_MAX);
        self.stretch_y = None;
        self.pan = Vec2::new(margin - origin.x * self.zoom, margin - origin.y * self.zoom);
    }
}

pub fn page_origin(col: i32, row: i32, page_w: f32, page_h: f32, gap: f32) -> Vec2 {
    Vec2::new(col as f32 * (page_w + gap), row as f32 * (page_h + gap))
}

/// Page whose sheet contains `p`, or the nearest sheet if `p` sits in a gutter.
pub fn page_at(p: Pos2, cells: &[(i32, i32)], page_w: f32, page_h: f32, gap: f32) -> usize {
    if cells.is_empty() {
        return 0;
    }
    let w = page_w.max(1.0);
    let h = page_h.max(1.0);
    let mut best = 0;
    let mut best_d = f32::MAX;
    for (i, &(col, row)) in cells.iter().enumerate() {
        let o = page_origin(col, row, w, h, gap);
        let r = Rect::from_min_size(Pos2::new(o.x, o.y), Vec2::new(w, h));
        if r.contains(p) {
            return i;
        }
        let cx = p.x.clamp(r.min.x, r.max.x);
        let cy = p.y.clamp(r.min.y, r.max.y);
        let d = (p.x - cx).hypot(p.y - cy);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{PAGE_GAP, PAGE_H, PAGE_W};

    #[test]
    fn origin_is_cell_times_stride() {
        let o = page_origin(1, 2, PAGE_W, PAGE_H, PAGE_GAP);
        assert_eq!(o.x, PAGE_W + PAGE_GAP);
        assert_eq!(o.y, 2.0 * (PAGE_H + PAGE_GAP));
    }

    #[test]
    fn page_at_hits_sheet_then_gutter() {
        let cells = [(0, 0), (1, 0), (0, 1)];
        let on_right = Pos2::new(PAGE_W + PAGE_GAP + 10.0, 10.0);
        assert_eq!(page_at(on_right, &cells, PAGE_W, PAGE_H, PAGE_GAP), 1);
        let gutter = Pos2::new(PAGE_W + PAGE_GAP * 0.5, 10.0);
        assert_eq!(page_at(gutter, &cells, PAGE_W, PAGE_H, PAGE_GAP), 0);
        let below = Pos2::new(10.0, PAGE_H + PAGE_GAP + 10.0);
        assert_eq!(page_at(below, &cells, PAGE_W, PAGE_H, PAGE_GAP), 2);
    }

    #[test]
    fn linked_origin_has_no_gap() {
        let o = page_origin(1, 0, PAGE_W, PAGE_H, 0.0);
        assert_eq!(o.x, PAGE_W);
        assert_eq!(o.y, 0.0);
    }

    #[test]
    fn free_resize_preserves_document_center_and_zoom() {
        let mut camera = Camera {
            pan: Vec2::new(-620.0, 73.0),
            zoom: 2.3,
            stretch_y: None,
        };
        let mut rect = Rect::from_min_size(Pos2::new(12.0, 64.0), Vec2::new(1200.0, 760.0));
        let focus = camera.to_paper(rect.center(), rect);
        for size in [
            Vec2::new(820.0, 1100.0),
            Vec2::new(1900.0, 600.0),
            Vec2::new(1200.0, 760.0),
        ] {
            let next = Rect::from_min_size(Pos2::new(100.0, 32.0), size);
            camera.resize_viewport(rect, next, None);
            assert!(camera.to_paper(next.center(), next).distance(focus) < 0.001);
            assert_eq!(camera.zoom, 2.3);
            rect = next;
        }
    }

    #[test]
    fn fitted_resize_zooms_without_changing_page_aspect() {
        for (page_w, page_h) in [(PAGE_W, PAGE_H), (PAGE_H, PAGE_W)] {
            let origin = page_origin(2, -1, page_w, page_h, 0.0);
            let size = Vec2::new(page_w, page_h);
            let mut camera = Camera::default();
            let mut previous = Rect::ZERO;
            for extent in [
                Vec2::new(800.0, 540.0),
                Vec2::new(1700.0, 920.0),
                Vec2::new(820.0, 1180.0),
            ] {
                let rect = Rect::from_min_size(Pos2::new(18.0, 76.0), extent);
                camera.resize_viewport(previous, rect, Some((origin, size)));
                assert!(camera.stretch_y.is_none());
                let page = Rect::from_min_max(
                    camera.to_screen(origin.to_pos2(), rect),
                    camera.to_screen((origin + size).to_pos2(), rect),
                );
                assert!((page.width() / page.height() - page_w / page_h).abs() < 0.002);
                assert!(page.min.x <= rect.min.x + 0.05 && page.max.x >= rect.max.x - 0.05);
                assert!(page.min.y <= rect.min.y + 0.05 && page.max.y >= rect.max.y - 0.05);
                let fills_x = (page.width() - rect.width()).abs() < 0.05;
                let fills_y = (page.height() - rect.height()).abs() < 0.05;
                assert!(fills_x || fills_y, "{page:?} vs {rect:?}");
                if !fills_x {
                    assert!(page.min.x < rect.min.x && page.max.x > rect.max.x);
                    assert!((page.center().x - rect.center().x).abs() < 0.05);
                }
                if !fills_y {
                    assert!(page.min.y < rect.min.y && page.max.y > rect.max.y);
                    assert!((page.center().y - rect.center().y).abs() < 0.05);
                }
                previous = rect;
            }
        }
    }

    #[test]
    fn invalid_viewports_do_not_modify_camera() {
        let mut camera = Camera {
            pan: Vec2::new(71.0, -144.0),
            zoom: 1.7,
            stretch_y: None,
        };
        let old = camera;
        let previous = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        for rect in [
            Rect::ZERO,
            Rect::NOTHING,
            Rect::EVERYTHING,
            Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 5.0)),
        ] {
            camera.resize_viewport(
                previous,
                rect,
                Some((Vec2::ZERO, Vec2::new(PAGE_W, PAGE_H))),
            );
            assert_eq!(camera.pan, old.pan);
            assert_eq!(camera.zoom, old.zoom);
        }
    }
}
