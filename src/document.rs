use std::collections::{BTreeMap, BTreeSet};

use crate::ink::{InkPoint, InkStroke, Nib};
use chrono::{DateTime, Utc};
use egui::{Color32, Pos2, Vec2};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Landscape A4 sheet, independent of the window.
pub const PAGE_W: f32 = 1123.0;
pub const PAGE_H: f32 = 794.0;
pub const PAGE_GAP: f32 = 56.0;
pub const CORNELL_CUE: f32 = 220.0;
pub const CORNELL_SUMMARY: f32 = 120.0;

fn default_page_w() -> f32 {
    PAGE_W
}
fn default_page_h() -> f32 {
    PAGE_H
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SheetJoin {
    #[default]
    Linked,
    Separate,
}

impl SheetJoin {
    pub fn gap(self) -> f32 {
        match self {
            SheetJoin::Linked => 0.0,
            SheetJoin::Separate => PAGE_GAP,
        }
    }
}

/// One “+” that would create the unit at `(dest_col, dest_row)`.
/// `center` is true when several edges share that hole (diagonal / gap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitTab {
    pub dest_col: i32,
    pub dest_row: i32,
    pub src_col: i32,
    pub src_row: i32,
    pub dcol: i32,
    pub drow: i32,
    pub center: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaperKind {
    #[serde(alias = "Slate")]
    Blank,
    Lined,
    Grid,
    Dots,
    #[default]
    Millimetre,
    Cornell,
}

impl PaperKind {
    pub const ALL: [PaperKind; 6] = [
        PaperKind::Blank,
        PaperKind::Lined,
        PaperKind::Grid,
        PaperKind::Dots,
        PaperKind::Millimetre,
        PaperKind::Cornell,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PaperKind::Blank => "Blank",
            PaperKind::Lined => "Lined",
            PaperKind::Grid => "Grid",
            PaperKind::Dots => "Dots",
            PaperKind::Millimetre => "Millimeter",
            PaperKind::Cornell => "Cornell",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            PaperKind::Blank => PaperKind::Lined,
            PaperKind::Lined => PaperKind::Grid,
            PaperKind::Grid => PaperKind::Dots,
            PaperKind::Dots => PaperKind::Millimetre,
            PaperKind::Millimetre => PaperKind::Cornell,
            PaperKind::Cornell => PaperKind::Blank,
        }
    }

    /// Horizontal rules the highlighter can cling to: `(first_y, step, last_y)`.
    pub fn highlight_rules(self, page_h: f32) -> Option<(f32, f32, f32)> {
        match self {
            PaperKind::Lined => Some((88.0, 28.0, page_h - 24.0)),
            PaperKind::Cornell => Some((88.0, 28.0, page_h - CORNELL_SUMMARY - 8.0)),
            PaperKind::Grid => Some((24.0, 24.0, page_h - 24.0)),
            PaperKind::Millimetre => Some((40.0, 8.0, page_h - 20.0)),
            PaperKind::Blank | PaperKind::Dots => None,
        }
    }

    pub fn has_margin_punches(self) -> bool {
        matches!(self, PaperKind::Lined | PaperKind::Cornell)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaperTone {
    White,
    #[default]
    Default,
    Black,
}

/// Closest ruled y, or `y` if the paper has no rules.
/// Row that owns a rule at paper `y`. A line on the shared edge stays on the page above.
pub fn rule_row(y: f32, page_h: f32) -> i32 {
    let h = page_h.max(1.0);
    let row = (y / h).floor() as i32;
    let into = y - row as f32 * h;
    if into <= 0.05 && y.abs() > 0.05 {
        row - 1
    } else {
        row
    }
}

/// Inclusive column runs on `row`, left to right.
pub fn col_runs_on_row(cells: &[(i32, i32)], row: i32) -> Vec<(i32, i32)> {
    let mut cols: Vec<i32> = cells
        .iter()
        .filter(|(_, r)| *r == row)
        .map(|(c, _)| *c)
        .collect();
    cols.sort_unstable();
    cols.dedup();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < cols.len() {
        let start = cols[i];
        let mut end = start;
        i += 1;
        while i < cols.len() && cols[i] == end + 1 {
            end = cols[i];
            i += 1;
        }
        runs.push((start, end));
    }
    runs
}

/// Inclusive row runs on `col`, top to bottom.
pub fn row_runs_on_col(cells: &[(i32, i32)], col: i32) -> Vec<(i32, i32)> {
    let mut rows: Vec<i32> = cells
        .iter()
        .filter(|(c, _)| *c == col)
        .map(|(_, r)| *r)
        .collect();
    rows.sort_unstable();
    rows.dedup();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let start = rows[i];
        let mut end = start;
        i += 1;
        while i < rows.len() && rows[i] == end + 1 {
            end = rows[i];
            i += 1;
        }
        runs.push((start, end));
    }
    runs
}

pub fn snap_rule_y(kind: PaperKind, page_h: f32, y: f32, extra: &[f32]) -> f32 {
    let mut best = y;
    let mut best_d = f32::MAX;
    let consider = |candidate: f32, best: &mut f32, best_d: &mut f32| {
        let d = (candidate - y).abs();
        if d < *best_d {
            *best_d = d;
            *best = candidate;
        }
    };
    if let Some((y0, step, last)) = kind.highlight_rules(page_h) {
        if step > 0.5 {
            let mut line = y0 + ((y - y0) / step).round() * step;
            if line < y0 {
                line = y0;
            }
            if line > last {
                line = last;
            }
            consider(line, &mut best, &mut best_d);
        }
    }
    for &line in extra {
        consider(line, &mut best, &mut best_d);
    }
    if (extra.is_empty() && best_d > 22.0) || best_d > 28.0 {
        y
    } else {
        best
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextBox {
    pub id: Uuid,
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub text: String,
    pub size_pt: f32,
    pub color: [u8; 4],
    #[serde(default)]
    pub sticky: bool,
}

impl TextBox {
    pub fn new(pos: Pos2, color: Color32) -> Self {
        Self {
            id: Uuid::new_v4(),
            pos: [pos.x, pos.y],
            size: [280.0, 80.0],
            text: String::new(),
            size_pt: 18.0,
            color: [color.r(), color.g(), color.b(), color.a()],
            sticky: false,
        }
    }

    pub fn sticky(pos: Pos2, color: Color32) -> Self {
        Self {
            size: [168.0, 168.0],
            size_pt: 16.0,
            sticky: true,
            ..Self::new(pos, color)
        }
    }

    pub fn min(&self) -> Pos2 {
        Pos2::new(self.pos[0], self.pos[1])
    }

    pub fn rect(&self) -> egui::Rect {
        egui::Rect::from_min_size(self.min(), Vec2::new(self.size[0], self.size[1]))
    }

    pub fn color32(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.color[0], self.color[1], self.color[2], self.color[3])
    }

    pub fn translate(&mut self, d: Vec2) {
        self.pos[0] += d.x;
        self.pos[1] += d.y;
    }

    pub fn contains(&self, p: Pos2) -> bool {
        self.rect().contains(p)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageObj {
    pub id: Uuid,
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub file: String,
}

impl ImageObj {
    pub fn min(&self) -> Pos2 {
        Pos2::new(self.pos[0], self.pos[1])
    }

    pub fn rect(&self) -> egui::Rect {
        egui::Rect::from_min_size(self.min(), Vec2::new(self.size[0], self.size[1]))
    }

    pub fn translate(&mut self, d: Vec2) {
        self.pos[0] += d.x;
        self.pos[1] += d.y;
    }
}

/// A non-editable page background: a copied PDF page or a placed image.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageBackground {
    /// Path relative to the notebook directory (`source.pdf`, `media/….png`).
    pub file: String,
    /// 0-based page when `file` is a PDF.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf_page: Option<u32>,
    /// Source width and height, in PDF points or image pixels, for aspect-fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_size: Option<[f32; 2]>,
}

impl PageBackground {
    /// Where the background sits inside a page of `page_w` × `page_h`.
    pub fn fit(&self, page_w: f32, page_h: f32) -> [f32; 4] {
        match self.source_size {
            Some([w, h]) => contain_rect(page_w, page_h, w, h),
            None => [0.0, 0.0, page_w.max(0.0), page_h.max(0.0)],
        }
    }
}

/// Fit `src` inside `dst`, centered, without stretching.
pub fn contain_rect(dst_w: f32, dst_h: f32, src_w: f32, src_h: f32) -> [f32; 4] {
    if !(dst_w > 0.0 && dst_h > 0.0 && src_w > 0.0 && src_h > 0.0) {
        return [0.0, 0.0, dst_w.max(0.0), dst_h.max(0.0)];
    }
    let s = (dst_w / src_w).min(dst_h / src_h);
    let w = src_w * s;
    let h = src_h * s;
    if (dst_w - w).abs() < 0.75 && (dst_h - h).abs() < 0.75 {
        return [0.0, 0.0, dst_w, dst_h];
    }
    [(dst_w - w) * 0.5, (dst_h - h) * 0.5, w, h]
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Page {
    pub strokes: Vec<InkStroke>,
    pub texts: Vec<TextBox>,
    pub images: Vec<ImageObj>,
    #[serde(default)]
    pub col: i32,
    #[serde(default)]
    pub row: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<PageBackground>,
}

impl Page {
    pub fn at(col: i32, row: i32) -> Self {
        Self {
            col,
            row,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub paper: PaperKind,
    #[serde(default)]
    pub paper_tone: PaperTone,
    pub cover: u8,
    #[serde(default)]
    pub emoji: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub pinned: bool,
    pub pages: Vec<Page>,
    #[serde(default = "default_page_w")]
    pub page_w: f32,
    #[serde(default = "default_page_h")]
    pub page_h: f32,
    #[serde(default)]
    pub sheet_join: SheetJoin,
}

impl Note {
    pub fn blank(title: impl Into<String>, cover: u8) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            paper: PaperKind::Millimetre,
            paper_tone: PaperTone::Default,
            cover,
            emoji: String::new(),
            created: now,
            updated: now,
            pinned: false,
            pages: vec![Page::default()],
            page_w: PAGE_W,
            page_h: PAGE_H,
            sheet_join: SheetJoin::Linked,
        }
    }

    pub fn page_size(&self) -> (f32, f32) {
        (self.page_w.max(1.0), self.page_h.max(1.0))
    }

    /// Page chrome for one paint, without stroke points or ribbon meshes.
    pub fn clone_for_paint(&self) -> Self {
        Self {
            id: self.id,
            title: self.title.clone(),
            paper: self.paper,
            paper_tone: self.paper_tone,
            cover: self.cover,
            emoji: self.emoji.clone(),
            created: self.created,
            updated: self.updated,
            pinned: self.pinned,
            page_w: self.page_w,
            page_h: self.page_h,
            sheet_join: self.sheet_join,
            pages: self
                .pages
                .iter()
                .map(|page| Page {
                    strokes: Vec::new(),
                    texts: page.texts.clone(),
                    images: page.images.clone(),
                    col: page.col,
                    row: page.row,
                    background: page.background.clone(),
                })
                .collect(),
        }
    }

    pub fn touch(&mut self) {
        self.updated = Utc::now();
    }

    pub fn cell_taken(&self, col: i32, row: i32) -> bool {
        self.pages.iter().any(|p| p.col == col && p.row == row)
    }

    /// Typed text and stickies, for the shelf search (not ink, not images).
    pub fn search_body(&self) -> String {
        let mut out = String::new();
        for page in &self.pages {
            for box_ in &page.texts {
                let t = box_.text.trim();
                if t.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
        out
    }

    /// Unit cells covered by the notebook, including a grown linked A4 sheet.
    pub fn unit_cells(&self) -> Vec<(i32, i32)> {
        if self.is_grown_single() {
            let p = &self.pages[0];
            let cols = tiles_along(self.page_w, PAGE_W);
            let rows = tiles_along(self.page_h, PAGE_H);
            let mut cells = Vec::with_capacity((cols * rows) as usize);
            for r in 0..rows {
                for c in 0..cols {
                    cells.push((p.col + c, p.row + r));
                }
            }
            return cells;
        }
        self.pages.iter().map(|p| (p.col, p.row)).collect()
    }

    pub fn unit_occupied(&self, col: i32, row: i32) -> bool {
        if self.is_grown_single() {
            self.unit_cells().contains(&(col, row))
        } else {
            self.cell_taken(col, row)
        }
    }

    /// Free sides of each unit cell: `(col, row, dcol, drow)`.
    pub fn free_unit_edges(&self) -> Vec<(i32, i32, i32, i32)> {
        let cells = self.unit_cells();
        let occupied: BTreeSet<_> = cells.iter().copied().collect();
        let mut out = Vec::new();
        for &(col, row) in &cells {
            for (dcol, drow) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                if !occupied.contains(&(col + dcol, row + drow)) {
                    out.push((col, row, dcol, drow));
                }
            }
        }
        out
    }

    /// One tab per empty destination. Shared holes (diagonal neighbors) get a center tab.
    pub fn unit_tabs(&self) -> Vec<UnitTab> {
        type Edge = (i32, i32, i32, i32);
        let mut by_dest: BTreeMap<(i32, i32), Vec<Edge>> = BTreeMap::new();
        for (col, row, dcol, drow) in self.free_unit_edges() {
            by_dest
                .entry((col + dcol, row + drow))
                .or_default()
                .push((col, row, dcol, drow));
        }
        by_dest
            .into_iter()
            .map(|(dest, srcs)| {
                let (src_col, src_row, dcol, drow) = srcs[0];
                UnitTab {
                    dest_col: dest.0,
                    dest_row: dest.1,
                    src_col,
                    src_row,
                    dcol,
                    drow,
                    center: srcs.len() >= 2,
                }
            })
            .collect()
    }

    pub fn is_grown_single(&self) -> bool {
        if self.pages.len() != 1 {
            return false;
        }
        // A PDF or image sheet is one page, not a mosaic of A4 units.
        if self.pages[0].background.is_some() {
            return false;
        }
        tiles_along(self.page_w, PAGE_W) > 1 || tiles_along(self.page_h, PAGE_H) > 1
    }

    /// Splits a grown linked sheet into unit pages, keeping `sheet_join`.
    pub fn explode_grown_units(&mut self) {
        self.split_grown_to_unit_pages();
    }

    pub fn add_unit_at(&mut self, col: i32, row: i32) -> bool {
        if self.unit_occupied(col, row) {
            return false;
        }
        if self.is_grown_single() {
            self.explode_grown_units();
        }
        self.pages.push(Page::at(col, row));
        true
    }

    pub fn can_tear_unit(&self) -> bool {
        self.unit_cells().len() > 1
    }

    /// Tears out one unit. The last sheet stays.
    pub fn remove_unit(&mut self, col: i32, row: i32) -> bool {
        if !self.can_tear_unit() || !self.unit_occupied(col, row) {
            return false;
        }
        if self.is_grown_single() {
            self.explode_grown_units();
        }
        self.pages.retain(|p| !(p.col == col && p.row == row));
        if self.pages.is_empty() {
            self.pages.push(Page::default());
        }
        true
    }

    /// Move a unit to an empty neighbour slot, or swap with the unit already there.
    pub fn move_unit(&mut self, from_col: i32, from_row: i32, to_col: i32, to_row: i32) -> bool {
        if from_col == to_col && from_row == to_row {
            return false;
        }
        if !self.cell_taken(from_col, from_row) {
            return false;
        }
        if self.is_grown_single() {
            self.explode_grown_units();
        }
        let from_i = self
            .pages
            .iter()
            .position(|p| p.col == from_col && p.row == from_row);
        let Some(from_i) = from_i else {
            return false;
        };
        if let Some(to_i) = self
            .pages
            .iter()
            .position(|p| p.col == to_col && p.row == to_row)
        {
            self.pages[from_i].col = to_col;
            self.pages[from_i].row = to_row;
            self.pages[to_i].col = from_col;
            self.pages[to_i].row = from_row;
            return true;
        }
        // Empty destination must be a defined neighbour slot (current + tab dest).
        let allowed = self
            .unit_tabs()
            .iter()
            .any(|t| t.dest_col == to_col && t.dest_row == to_row);
        if !allowed {
            return false;
        }
        self.pages[from_i].col = to_col;
        self.pages[from_i].row = to_row;
        true
    }

    /// Old notes stacked pages by vec index with no col/row. Spread them vertically.
    pub fn normalize_page_grid(&mut self) {
        if self.pages.len() <= 1 {
            return;
        }
        if self.pages.iter().all(|p| p.col == 0 && p.row == 0) {
            for (i, p) in self.pages.iter_mut().enumerate() {
                p.row = i as i32;
            }
        }
    }

    /// Linked keeps the same cells (holes stay holes) and only changes the join.
    /// A grown rectangle is split into unit pages when switching to Separate.
    /// Custom sheet size (PDF pages) stays put.
    pub fn apply_sheet_join(&mut self, to: SheetJoin) {
        if self.pages.is_empty() {
            self.pages.push(Page::default());
        }
        if to == SheetJoin::Separate && self.is_grown_single() {
            self.split_grown_to_unit_pages();
        }
        self.sheet_join = to;
    }

    /// Size the sheet to the PDF pages. Restores notes whose size was reset to A4.
    pub fn sync_pdf_sheet_size(&mut self) -> bool {
        let mut max_w = 0.0f32;
        let mut max_h = 0.0f32;
        let mut any = false;
        for page in &self.pages {
            let Some(bg) = page.background.as_ref() else {
                continue;
            };
            if bg.pdf_page.is_none() {
                continue;
            }
            let Some([w, h]) = bg.source_size else {
                continue;
            };
            let (lw, lh) = crate::pdf::logical_size(w, h);
            max_w = max_w.max(lw);
            max_h = max_h.max(lh);
            any = true;
        }
        if !any {
            return false;
        }
        if (self.page_w - max_w).abs() < 1.0 && (self.page_h - max_h).abs() < 1.0 {
            return false;
        }
        let old_w = self.page_w;
        let old_h = self.page_h;
        for page in &mut self.pages {
            let Some(bg) = page.background.clone() else {
                continue;
            };
            if bg.pdf_page.is_none() {
                continue;
            }
            let from = bg.fit(old_w, old_h);
            let to = bg.fit(max_w, max_h);
            remap_page_content(page, from, to);
        }
        self.page_w = max_w;
        self.page_h = max_h;
        true
    }

    fn split_grown_to_unit_pages(&mut self) {
        if !self.is_grown_single() {
            return;
        }
        let p = self.pages[0].clone();
        let tw = PAGE_W;
        let th = PAGE_H;
        let cols = tiles_along(self.page_w, tw);
        let rows = tiles_along(self.page_h, th);
        let mut tiles: BTreeMap<(i32, i32), Page> = BTreeMap::new();
        for r in 0..rows {
            for c in 0..cols {
                tiles.insert((p.col + c, p.row + r), Page::at(p.col + c, p.row + r));
            }
        }
        for s in &p.strokes {
            for (c, r, mut piece) in split_stroke_across_tiles(s, tw, th) {
                let (c, r) = clamp_cell(c, r, cols, rows);
                piece.translate(Vec2::new(-(c as f32 * tw), -(r as f32 * th)));
                let key = (p.col + c, p.row + r);
                tiles
                    .entry(key)
                    .or_insert_with(|| Page::at(key.0, key.1))
                    .strokes
                    .push(piece);
            }
        }
        for mut t in p.texts {
            let (c, r) = clamp_cell(
                (t.pos[0] / tw).floor() as i32,
                (t.pos[1] / th).floor() as i32,
                cols,
                rows,
            );
            t.pos[0] -= c as f32 * tw;
            t.pos[1] -= r as f32 * th;
            let key = (p.col + c, p.row + r);
            tiles
                .entry(key)
                .or_insert_with(|| Page::at(key.0, key.1))
                .texts
                .push(t);
        }
        for mut im in p.images {
            let (c, r) = clamp_cell(
                (im.pos[0] / tw).floor() as i32,
                (im.pos[1] / th).floor() as i32,
                cols,
                rows,
            );
            im.pos[0] -= c as f32 * tw;
            im.pos[1] -= r as f32 * th;
            let key = (p.col + c, p.row + r);
            tiles
                .entry(key)
                .or_insert_with(|| Page::at(key.0, key.1))
                .images
                .push(im);
        }
        if let Some(bg) = p.background.clone() {
            let key = (p.col, p.row);
            if let Some(home) = tiles.get_mut(&key) {
                home.background = Some(bg);
            }
        }
        self.page_w = tw;
        self.page_h = th;
        self.pages = tiles.into_values().collect();
    }
}

fn remap_page_content(page: &mut Page, from: [f32; 4], to: [f32; 4]) {
    let [fx, fy, fw, fh] = from;
    let [tx, ty, tw, th] = to;
    if fw < 1.0 || fh < 1.0 {
        return;
    }
    let sx = tw / fw;
    let sy = th / fh;
    if (sx - 1.0).abs() < 0.001
        && (sy - 1.0).abs() < 0.001
        && (fx - tx).abs() < 0.5
        && (fy - ty).abs() < 0.5
    {
        return;
    }
    let map = |x: f32, y: f32| (tx + (x - fx) * sx, ty + (y - fy) * sy);
    let avg = (sx + sy) * 0.5;
    for stroke in &mut page.strokes {
        for point in &mut stroke.points {
            let (x, y) = map(point.x, point.y);
            point.x = x;
            point.y = y;
        }
        stroke.width *= avg;
        stroke.mesh = None;
    }
    for text in &mut page.texts {
        let (x, y) = map(text.pos[0], text.pos[1]);
        text.pos = [x, y];
        text.size[0] *= sx;
        text.size[1] *= sy;
        text.size_pt *= avg;
    }
    for image in &mut page.images {
        let (x, y) = map(image.pos[0], image.pos[1]);
        image.pos = [x, y];
        image.size[0] *= sx;
        image.size[1] *= sy;
    }
}

fn tiles_along(len: f32, unit: f32) -> i32 {
    let unit = unit.max(1.0);
    ((len / unit) - 1e-3).ceil().max(1.0) as i32
}

fn clamp_cell(c: i32, r: i32, cols: i32, rows: i32) -> (i32, i32) {
    (
        c.clamp(0, cols.saturating_sub(1).max(0)),
        r.clamp(0, rows.saturating_sub(1).max(0)),
    )
}

fn split_stroke_across_tiles(stroke: &InkStroke, tw: f32, th: f32) -> Vec<(i32, i32, InkStroke)> {
    if stroke.points.is_empty() {
        return Vec::new();
    }
    let cell = |p: InkPoint| ((p.x / tw).floor() as i32, (p.y / th).floor() as i32);
    let mut out = Vec::new();
    let mut cur_cell = cell(stroke.points[0]);
    let mut cur = InkStroke::new(stroke.nib, stroke.color32(), stroke.width);
    cur.sticky = stroke.sticky;
    cur.points.push(stroke.points[0]);
    for pair in stroke.points.windows(2) {
        let a = pair[0];
        let b = pair[1];
        let mut hits: Vec<(f32, InkPoint)> = Vec::new();
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        if dx.abs() > 1e-8 {
            let lo = a.x.min(b.x);
            let hi = a.x.max(b.x);
            let k0 = ((lo + 1e-4) / tw).ceil() as i32;
            let k1 = ((hi - 1e-4) / tw).floor() as i32;
            for k in k0..=k1 {
                let x = k as f32 * tw;
                let t = (x - a.x) / dx;
                if t > 1e-6 && t < 1.0 - 1e-6 {
                    let y = a.y + dy * t;
                    let p = a.p + (b.p - a.p) * t;
                    hits.push((t, InkPoint::new(Pos2::new(x, y), p)));
                }
            }
        }
        if dy.abs() > 1e-8 {
            let lo = a.y.min(b.y);
            let hi = a.y.max(b.y);
            let k0 = ((lo + 1e-4) / th).ceil() as i32;
            let k1 = ((hi - 1e-4) / th).floor() as i32;
            for k in k0..=k1 {
                let y = k as f32 * th;
                let t = (y - a.y) / dy;
                if t > 1e-6 && t < 1.0 - 1e-6 {
                    let x = a.x + dx * t;
                    let p = a.p + (b.p - a.p) * t;
                    hits.push((t, InkPoint::new(Pos2::new(x, y), p)));
                }
            }
        }
        hits.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
        hits.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-5);
        for (_, p) in hits {
            cur.points.push(p);
            out.push((
                cur_cell.0,
                cur_cell.1,
                std::mem::replace(&mut cur, {
                    let mut neu = InkStroke::new(stroke.nib, stroke.color32(), stroke.width);
                    neu.sticky = stroke.sticky;
                    neu
                }),
            ));
            cur.points.push(p);
            cur_cell = cell(p);
        }
        cur.points.push(b);
        cur_cell = cell(b);
    }
    if !cur.points.is_empty() {
        out.push((cur_cell.0, cur_cell.1, cur));
    }
    out
}

pub fn stroke_from_polyline(pts: &[[f32; 2]], nib: Nib, color: Color32, width: f32) -> InkStroke {
    let mut s = InkStroke::new(nib, color, width);
    for p in pts {
        s.points.push(InkPoint::new(Pos2::new(p[0], p[1]), 0.85));
    }
    s
}

pub fn squiggle(from: Pos2, to: Pos2, amp: f32, n: usize) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let base = from.lerp(to, t);
        let dir = (to - from).normalized();
        let nrm = Vec2::new(-dir.y, dir.x);
        let wobble = (t * std::f32::consts::PI * 3.0).sin() * amp * (1.0 - (t - 0.5).abs());
        let p = base + nrm * wobble;
        out.push([p.x, p.y]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_spreads_legacy_stack() {
        let mut n = Note::blank("t", 0);
        n.pages.push(Page::default());
        n.pages.push(Page::default());
        n.normalize_page_grid();
        assert_eq!(
            n.pages.iter().map(|p| (p.col, p.row)).collect::<Vec<_>>(),
            vec![(0, 0), (0, 1), (0, 2)]
        );
    }

    #[test]
    fn normalize_keeps_placed_grid() {
        let mut n = Note::blank("t", 0);
        n.pages = vec![Page::at(0, 0), Page::at(1, 0), Page::at(0, 1)];
        n.normalize_page_grid();
        assert_eq!(
            n.pages.iter().map(|p| (p.col, p.row)).collect::<Vec<_>>(),
            vec![(0, 0), (1, 0), (0, 1)]
        );
    }

    #[test]
    fn sheet_join_default_is_linked() {
        let n = Note::blank("t", 0);
        assert_eq!(n.sheet_join, SheetJoin::Linked);
        let parsed: Note = serde_json::from_str(
            r#"{"id":"00000000-0000-0000-0000-000000000000","title":"t","paper":"Blank","cover":0,"created":"2020-01-01T00:00:00Z","updated":"2020-01-01T00:00:00Z","pinned":false,"pages":[]}"#,
        )
        .unwrap();
        assert_eq!(parsed.sheet_join, SheetJoin::Linked);
        assert!(parsed.pages.is_empty());
    }

    fn mark(x: f32, y: f32) -> InkStroke {
        let mut s = InkStroke::new(Nib::Fineliner, Color32::BLACK, 2.0);
        s.points.push(InkPoint::new(Pos2::new(x, y), 1.0));
        s
    }

    #[test]
    fn separate_splits_grown_sheet_into_unit_pages() {
        let mut n = Note::blank("t", 0);
        n.page_w = PAGE_W * 2.0;
        n.page_h = PAGE_H;
        n.pages[0].strokes.push(mark(10.0, 12.0));
        n.pages[0].strokes.push(mark(PAGE_W + 40.0, 8.0));
        n.apply_sheet_join(SheetJoin::Separate);
        assert_eq!(n.sheet_join, SheetJoin::Separate);
        assert_eq!(n.page_w, PAGE_W);
        assert_eq!(n.page_h, PAGE_H);
        assert_eq!(n.pages.len(), 2);
        let cells: Vec<_> = n.pages.iter().map(|p| (p.col, p.row)).collect();
        assert_eq!(cells, vec![(0, 0), (1, 0)]);
        assert_eq!(n.pages[0].strokes[0].points[0].x, 10.0);
        assert!((n.pages[1].strokes[0].points[0].x - 40.0).abs() < 0.01);
    }

    #[test]
    fn linked_keeps_holes() {
        let mut n = Note::blank("t", 0);
        n.sheet_join = SheetJoin::Separate;
        n.pages = vec![Page::at(0, 0), Page::at(1, 0), Page::at(0, 1)];
        n.pages[1].strokes.push(mark(40.0, 8.0));
        n.apply_sheet_join(SheetJoin::Linked);
        assert_eq!(n.sheet_join, SheetJoin::Linked);
        assert_eq!(n.page_w, PAGE_W);
        assert_eq!(n.page_h, PAGE_H);
        let mut cells: Vec<_> = n.pages.iter().map(|p| (p.col, p.row)).collect();
        cells.sort();
        assert_eq!(cells, vec![(0, 0), (0, 1), (1, 0)]);
        let right = n.pages.iter().find(|p| p.col == 1 && p.row == 0).unwrap();
        assert_eq!(right.strokes[0].points[0].x, 40.0);
    }

    #[test]
    fn unit_cells_pave_grown_sheet() {
        let mut n = Note::blank("t", 0);
        n.page_w = PAGE_W * 2.0;
        n.page_h = PAGE_H * 2.0;
        let mut cells = n.unit_cells();
        cells.sort();
        assert_eq!(cells, vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
        assert_eq!(n.free_unit_edges().len(), 8);
        assert!(n.unit_tabs().iter().all(|t| !t.center));
        assert_eq!(n.unit_tabs().len(), 8);
    }

    #[test]
    fn diagonal_pages_share_center_tabs() {
        let mut n = Note::blank("t", 0);
        n.sheet_join = SheetJoin::Separate;
        n.pages = vec![Page::at(0, 0), Page::at(1, 1)];
        let tabs = n.unit_tabs();
        let mut dests: Vec<_> = tabs.iter().map(|t| (t.dest_col, t.dest_row)).collect();
        dests.sort();
        dests.dedup();
        assert_eq!(dests.len(), tabs.len());
        let mut centers: Vec<_> = tabs
            .iter()
            .filter(|t| t.center)
            .map(|t| (t.dest_col, t.dest_row))
            .collect();
        centers.sort();
        assert_eq!(centers, vec![(0, 1), (1, 0)]);
        assert_eq!(tabs.len(), 6);
    }

    #[test]
    fn linked_add_right_on_strip_keeps_units() {
        let mut n = Note::blank("t", 0);
        n.page_w = PAGE_W * 2.0;
        n.pages[0].strokes.push(mark(10.0, 12.0));
        assert!(n.add_unit_at(2, 0));
        assert_eq!(n.sheet_join, SheetJoin::Linked);
        assert_eq!(n.page_w, PAGE_W);
        assert_eq!(n.page_h, PAGE_H);
        let mut cells: Vec<_> = n.pages.iter().map(|p| (p.col, p.row)).collect();
        cells.sort();
        assert_eq!(cells, vec![(0, 0), (1, 0), (2, 0)]);
        let home = n.pages.iter().find(|p| p.col == 0 && p.row == 0).unwrap();
        assert_eq!(home.strokes[0].points[0].x, 10.0);
    }

    #[test]
    fn linked_add_on_block_keeps_one_cell() {
        let mut n = Note::blank("t", 0);
        n.page_w = PAGE_W * 2.0;
        n.page_h = PAGE_H * 2.0;
        n.pages[0].strokes.push(mark(10.0, 12.0));
        assert!(n.add_unit_at(2, 0));
        assert_eq!(n.sheet_join, SheetJoin::Linked);
        assert_eq!(n.page_w, PAGE_W);
        assert_eq!(n.page_h, PAGE_H);
        let mut cells: Vec<_> = n.pages.iter().map(|p| (p.col, p.row)).collect();
        cells.sort();
        assert_eq!(cells, vec![(0, 0), (0, 1), (1, 0), (1, 1), (2, 0)]);
        let home = n.pages.iter().find(|p| p.col == 0 && p.row == 0).unwrap();
        assert_eq!(home.strokes[0].points[0].x, 10.0);
    }

    #[test]
    fn join_roundtrip_keeps_marks() {
        let mut n = Note::blank("t", 0);
        n.page_w = PAGE_W * 2.0;
        n.page_h = PAGE_H * 2.0;
        n.pages[0].strokes.push(mark(PAGE_W + 5.0, PAGE_H + 6.0));
        n.apply_sheet_join(SheetJoin::Separate);
        n.apply_sheet_join(SheetJoin::Linked);
        assert_eq!(n.sheet_join, SheetJoin::Linked);
        assert_eq!(n.pages.len(), 4);
        assert_eq!(n.page_w, PAGE_W);
        assert_eq!(n.page_h, PAGE_H);
        let p = n.pages.iter().find(|p| p.col == 1 && p.row == 1).unwrap();
        let pt = &p.strokes[0].points[0];
        assert!((pt.x - 5.0).abs() < 0.05);
        assert!((pt.y - 6.0).abs() < 0.05);
    }

    #[test]
    fn tear_unit_keeps_the_rest() {
        let mut n = Note::blank("t", 0);
        n.sheet_join = SheetJoin::Separate;
        n.pages = vec![Page::at(0, 0), Page::at(1, 0), Page::at(0, 1)];
        n.pages[1].strokes.push(mark(4.0, 5.0));
        assert!(n.remove_unit(0, 0));
        let mut cells: Vec<_> = n.pages.iter().map(|p| (p.col, p.row)).collect();
        cells.sort();
        assert_eq!(cells, vec![(0, 1), (1, 0)]);
        let right = n.pages.iter().find(|p| p.col == 1).unwrap();
        assert_eq!(right.strokes[0].points[0].x, 4.0);
    }

    #[test]
    fn tear_last_unit_is_refused() {
        let mut n = Note::blank("t", 0);
        assert!(!n.can_tear_unit());
        assert!(!n.remove_unit(0, 0));
        assert_eq!(n.pages.len(), 1);
    }

    #[test]
    fn move_unit_to_neighbour_and_swap() {
        let mut n = Note::blank("t", 0);
        n.sheet_join = SheetJoin::Separate;
        n.pages = vec![Page::at(0, 0), Page::at(1, 0)];
        n.pages[0].strokes.push(mark(1.0, 2.0));
        n.pages[1].strokes.push(mark(3.0, 4.0));
        assert!(n.move_unit(1, 0, 2, 0), "slide into a free neighbour slot");
        assert!(n.cell_taken(0, 0));
        assert!(!n.cell_taken(1, 0));
        assert!(n.cell_taken(2, 0));
        let moved = n.pages.iter().find(|p| p.col == 2).unwrap();
        assert_eq!(moved.strokes[0].points[0].x, 3.0);
        assert!(n.move_unit(0, 0, 2, 0), "swap with the occupied neighbour");
        let a = n.pages.iter().find(|p| p.col == 0).unwrap();
        let b = n.pages.iter().find(|p| p.col == 2).unwrap();
        assert_eq!(a.strokes[0].points[0].x, 3.0);
        assert_eq!(b.strokes[0].points[0].x, 1.0);
        assert!(!n.move_unit(0, 0, 5, 5), "far empty cells are not slots");
    }

    #[test]
    fn search_body_joins_typed_text_and_stickies() {
        let mut n = Note::blank("Mindset", 0);
        let mut typed = TextBox::new(Pos2::new(10.0, 10.0), Color32::BLACK);
        typed.text = "Fourier series".into();
        let mut sticky = TextBox::sticky(Pos2::new(40.0, 40.0), Color32::YELLOW);
        sticky.text = "  exam  ".into();
        n.pages[0].texts.push(typed);
        n.pages[0].texts.push(sticky);
        assert_eq!(n.search_body(), "Fourier series\nexam");
    }

    #[test]
    fn page_without_background_still_loads() {
        let page: Page = serde_json::from_str(r#"{"strokes":[],"texts":[],"images":[]}"#).unwrap();
        assert!(page.background.is_none());
    }

    #[test]
    fn contain_rect_letterboxes_a_wide_source() {
        let [x, y, w, h] = contain_rect(100.0, 100.0, 200.0, 100.0);
        assert!((w - 100.0).abs() < 0.01);
        assert!((h - 50.0).abs() < 0.01);
        assert!((x - 0.0).abs() < 0.01);
        assert!((y - 25.0).abs() < 0.01);
    }

    #[test]
    fn contain_rect_fills_when_the_aspect_matches() {
        let [x, y, w, h] = contain_rect(200.0, 100.0, 400.0, 200.0);
        assert_eq!([x, y, w, h], [0.0, 0.0, 200.0, 100.0]);
    }

    #[test]
    fn tear_from_grown_sheet_explodes() {
        let mut n = Note::blank("t", 0);
        n.page_w = PAGE_W * 2.0;
        n.pages[0].strokes.push(mark(10.0, 12.0));
        n.pages[0].strokes.push(mark(PAGE_W + 40.0, 8.0));
        assert!(n.remove_unit(1, 0));
        assert_eq!(n.pages.len(), 1);
        assert_eq!(n.pages[0].col, 0);
        assert_eq!(n.pages[0].strokes[0].points[0].x, 10.0);
    }

    fn pdf_page(w_pt: f32, h_pt: f32, page: u32) -> Page {
        let mut p = Page::at(0, page as i32);
        p.background = Some(PageBackground {
            file: "source.pdf".into(),
            pdf_page: Some(page),
            source_size: Some([w_pt, h_pt]),
        });
        p
    }

    #[test]
    fn pdf_sheet_is_not_a_grown_a4_mosaic() {
        let mut n = Note::blank("talk", 0);
        let (w, h) = crate::pdf::logical_size(603.0, 783.0);
        n.page_w = w;
        n.page_h = h;
        n.pages = vec![pdf_page(603.0, 783.0, 0)];
        assert!(!n.is_grown_single());
        assert_eq!(n.unit_cells(), vec![(0, 0)]);
    }

    #[test]
    fn joining_pdf_pages_keeps_their_size() {
        let mut n = Note::blank("talk", 0);
        let (w, h) = crate::pdf::logical_size(603.0, 783.0);
        n.page_w = w;
        n.page_h = h;
        n.sheet_join = SheetJoin::Separate;
        n.pages = vec![pdf_page(603.0, 783.0, 0), pdf_page(603.0, 783.0, 1)];
        n.apply_sheet_join(SheetJoin::Linked);
        assert!((n.page_w - w).abs() < 0.5);
        assert!((n.page_h - h).abs() < 0.5);
        n.remove_unit(0, 1);
        assert!((n.page_w - w).abs() < 0.5);
        assert!((n.page_h - h).abs() < 0.5);
    }

    #[test]
    fn sync_pdf_sheet_size_restores_portrait_from_a4() {
        let mut n = Note::blank("talk", 0);
        n.pages = vec![pdf_page(603.0, 783.0, 0), pdf_page(603.0, 783.0, 1)];
        assert!(n.sync_pdf_sheet_size());
        let (w, h) = crate::pdf::logical_size(603.0, 783.0);
        assert!((n.page_w - w).abs() < 0.5);
        assert!((n.page_h - h).abs() < 0.5);
        assert!(!n.sync_pdf_sheet_size());
    }

    #[test]
    fn paper_cycle_includes_cornell() {
        let mut k = PaperKind::Millimetre;
        k = k.cycle();
        assert_eq!(k, PaperKind::Cornell);
        assert_eq!(k.cycle(), PaperKind::Blank);
        assert_eq!(k.label(), "Cornell");
    }

    #[test]
    fn clone_for_paint_drops_strokes_and_keeps_text() {
        let mut n = Note::blank("t", 1);
        n.pages[0].strokes.push(stroke_from_polyline(
            &[[10.0, 10.0], [40.0, 12.0]],
            Nib::Fineliner,
            Color32::BLACK,
            2.0,
        ));
        n.pages[0]
            .texts
            .push(TextBox::new(Pos2::new(8.0, 8.0), Color32::BLACK));
        let paint = n.clone_for_paint();
        assert!(paint.pages[0].strokes.is_empty());
        assert_eq!(paint.pages[0].texts.len(), 1);
        assert_eq!(paint.id, n.id);
        assert_eq!(paint.paper, n.paper);
        assert_eq!(n.pages[0].strokes.len(), 1);
    }

    #[test]
    fn sticky_survives_json() {
        let mut n = Note::blank("t", 0);
        n.pages[0]
            .texts
            .push(TextBox::sticky(Pos2::new(12.0, 20.0), Color32::YELLOW));
        let s = serde_json::to_string(&n).unwrap();
        let back: Note = serde_json::from_str(&s).unwrap();
        assert!(back.pages[0].texts[0].sticky);
        let old: Note = serde_json::from_str(
            r#"{"id":"00000000-0000-0000-0000-000000000000","title":"t","paper":"Blank","cover":0,"created":"2020-01-01T00:00:00Z","updated":"2020-01-01T00:00:00Z","pinned":false,"pages":[]}"#,
        )
        .unwrap();
        assert_eq!(old.paper, PaperKind::Blank);
    }

    #[test]
    fn linked_rules_merge_adjacent_cells_and_keep_holes() {
        let cells = [(0, 0), (1, 0), (3, 0), (0, 1)];
        assert_eq!(col_runs_on_row(&cells, 0), vec![(0, 1), (3, 3)]);
        assert_eq!(col_runs_on_row(&cells, 1), vec![(0, 0)]);
        assert_eq!(row_runs_on_col(&cells, 0), vec![(0, 1)]);
        assert_eq!(rule_row(88.0, PAGE_H), 0);
        assert_eq!(rule_row(PAGE_H, PAGE_H), 0);
        assert_eq!(rule_row(PAGE_H + 10.0, PAGE_H), 1);
    }

    #[test]
    fn snap_rule_hits_lined_paper() {
        let y = snap_rule_y(PaperKind::Lined, PAGE_H, 88.0 + 13.0, &[]);
        assert!((y - 88.0).abs() < 0.1 || (y - 116.0).abs() < 0.1);
    }
}
