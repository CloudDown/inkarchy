#![allow(unknown_lints, float_literal_f32_fallback)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::*;
use egui::containers::scroll_area::{ScrollBarVisibility, ScrollSource};
use egui::epaint::Vertex;
use egui::style::ScrollAnimation;
use uuid::Uuid;

use crate::camera::{page_at, page_origin, Camera, ZOOM_STOPS};
use crate::document::{
    ImageObj, Note, PageBackground, PaperKind, SheetJoin, TextBox, PAGE_H, PAGE_W,
};
use crate::emoji::Atlas;
use crate::export::{self, MediaLoader};
use crate::ink::{
    default_width, draw_ants, erase_area, map_mesh, maybe_snap_shape, mixed_pressure, InkPoint,
    InkStroke, Nib, Tool,
};
use crate::library::{
    ensure_png, image_size, DockEdge, Library, SaveError, SHELF_COLS, SHELF_SLOTS, TRASH_SLOTS,
};
use crate::look::Look;
use crate::pressure::Pressure;
use crate::seed;
use crate::tablet::{PenSnapshot, TabletBridge};
use crate::undo::UndoStack;

#[cfg(test)]
mod tests;

const DOS_W: f32 = 128.0;
const DOS_H: f32 = 156.0;
const DOS_PAD: f32 = 10.0;
const PAPER_PEEK: f32 = 18.0;
const TITLE_ROW: f32 = 44.0;
const SHELF_GAP: f32 = 8.0;
const TOSS_DURATION: f64 = 0.30;
const TOSS_STAGGER: f64 = 0.035;
const BIN_PANEL_SPEED: f32 = 40.0;
const BIN_REVEAL_DURATION: f32 = 0.13;
const BIN_HOVER_DURATION: f32 = 0.09;
const BIN_DROP_RADIUS: f32 = 176.0;
const QUARTER_DISK_CENTROID: f32 = 4.0 / (3.0 * std::f32::consts::PI);

#[derive(Clone)]
enum Scene {
    Shelf { query: String },
    Desk,
}

#[derive(Clone, Copy)]
enum DosAct {
    Open,
    Select,
    Toggle,
    Range,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShelfDrop {
    Shelf(usize),
    Bin(usize),
}

struct Toss {
    id: Uuid,
    from: Pos2,
    cover: u8,
    emoji: String,
    t0: f64,
    delay: f64,
}

struct Toast {
    msg: String,
    until: f64,
}

#[derive(Clone, Copy)]
enum SelKind {
    Stroke,
    Text,
    Image,
}

#[derive(Clone, Copy)]
struct Sel {
    page: usize,
    id: Uuid,
    kind: SelKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SheetEdge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WheelPick {
    Pin,
    Mark,
    Color,
    Trash,
    Cloth(u8),
}

struct SpineWheel {
    id: Uuid,
    origin: Pos2,
    t0: f64,
    colors: bool,
    hover: Option<WheelPick>,
    secondary: bool,
    color_hold_t0: Option<f64>,
    touch_origin: Option<Pos2>,
}

#[derive(Clone, Copy)]
enum MoreAct {
    Open,
    Linked,
    Separate,
    Download,
    Trash,
    Fit,
    Paper,
}

pub struct CahierApp {
    look: Look,
    lib: Library,
    scene: Scene,
    note: Option<Note>,
    page_delete: Option<Vec<(i32, i32)>>,
    page_delete_view: Option<(Camera, Rect)>,
    tool: Tool,
    color_i: usize,
    /// Current ink (theme swatch or a free tint).
    ink: Color32,
    color_custom: bool,
    /// Watercolor tin (large palette) is open.
    tin_open: bool,
    tin_hue: f32,
    width: f32,
    last_ink: Tool,
    last_ink_width: f32,
    last_eraser: Tool,
    palm_grace_until: Option<Instant>,
    touch_grace_until: Option<Instant>,
    two_finger: Option<(f64, f32, f32)>,
    camera: Camera,
    undo: UndoStack,
    live: Option<(usize, InkStroke)>,
    /// Still nib: anchor, since when, and the shape already shown.
    shape_anchor: Option<Pos2>,
    shape_still: Option<Instant>,
    shape_preview: Option<InkStroke>,
    lasso: Vec<Pos2>,
    sel: Vec<Sel>,
    drag_last: Option<Pos2>,
    editing_text: Option<(usize, Uuid)>,
    dirty: bool,
    last_change: Instant,
    textures: HashMap<String, TextureHandle>,
    emoji: Atlas,
    emoji_tex: HashMap<char, TextureHandle>,
    pressure: Pressure,
    last_ptr: Option<(Pos2, Instant)>,
    toast: Option<Toast>,
    need_fit: bool,
    /// Cell kept fitted during viewport resizing; None means free navigation.
    fitted_cell: Option<(i32, i32)>,
    /// Page to frame in writing view (top of the sheet).
    land_page: Option<usize>,
    title_buf: String,
    dock_edge: DockEdge,
    dock_float: Option<Pos2>,
    dock_grab: Vec2,
    dock_moved: bool,
    canvas_rect: Rect,
    /// Click held on a sheet ear: do not lay ink.
    tab_held: bool,
    tablet: TabletBridge,
    live_from_pen: bool,
    /// Mouse cursor hidden (stylus in proximity).
    cursor_off: bool,
    /// Stylus was in proximity on the previous frame (for PointerGone).
    pen_was_prox: bool,
    /// Emoji picker open for this note.
    emoji_pick: Option<Uuid>,
    /// Filter inside the type case.
    emoji_query: String,
    /// Folder title currently being edited.
    rename_id: Option<Uuid>,
    rename_buf: String,
    /// Title position, so the editor can sit outside the scroll.
    rename_rect: Option<Rect>,
    /// The text field was just placed: give it the keyboard.
    text_focus: bool,
    /// Ink palette unfolded (panel beside the dock; dock size stays fixed).
    palette_open: bool,
    /// Stylus click that began on the chrome (not on the sheet).
    pen_ui_grab: bool,
    /// Open the image picker outside the click (system portal).
    pending_image: Option<(usize, Pos2)>,
    /// Open the file dialog outside the click (system portal).
    pending_open: bool,
    /// Paths sent by a second `cahier file.pdf` while this window is open.
    inbox: Option<crate::ipc::Inbox>,
    bg_textures: HashMap<String, TextureHandle>,
    bg_order: VecDeque<(String, usize)>,
    bg_bytes: usize,
    bg_failed: HashSet<String>,
    startup_toast: Option<String>,
    /// File format chooser shown after saving a note for download.
    export_picker_open: bool,
    /// Explorer-style selection on the shelf.
    shelf_sel: Vec<Uuid>,
    shelf_anchor: Option<Uuid>,
    /// Red bin row open at the bottom of the shelf.
    shelf_trash: bool,
    bin_panel_height: f32,
    bin_close_rect: Rect,
    /// Spine faces, for the selection rectangle.
    shelf_slots: Vec<(Uuid, Rect)>,
    /// Origin of the rectangle (None = no band).
    shelf_band: Option<Pos2>,
    shelf_band_now: Option<Pos2>,
    shelf_band_add: bool,
    shelf_band_armed: bool,
    shelf_band_base: Vec<Uuid>,
    /// Dragging spines toward the basket.
    shelf_haul: Option<Pos2>,
    shelf_haul_now: Option<Pos2>,
    shelf_haul_ids: Vec<Uuid>,
    shelf_haul_armed: bool,
    /// Grid cell while dragging.
    shelf_drop: Option<ShelfDrop>,
    /// Every shelf cell (occupied or empty).
    shelf_grid: Vec<Rect>,
    /// Bin row cells when the red zone is open.
    trash_grid: Vec<Rect>,
    shelf_toss: Vec<Toss>,
    trash_rect: Rect,
    trash_mouth: Pos2,
    trash_reveal: f32,
    shelf_fed: bool,
    /// Long-press radial menu on a spine.
    spine_wheel: Option<SpineWheel>,
    shelf_hold_t0: Option<f64>,
}

impl CahierApp {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        let look = Look::load();
        look.apply(&cc.egui_ctx);
        let mut lib = Library::open();
        seed::seed_if_needed(&mut lib);
        let mut app = Self::with_library(look, lib);
        if files.is_empty() {
            if let Ok(q) = std::env::var("CAHIER_OPEN") {
                let q = q.trim().to_lowercase();
                if !q.is_empty() {
                    if let Some(id) = app
                        .lib
                        .index
                        .notes
                        .iter()
                        .find(|m| m.title.to_lowercase().contains(&q))
                        .map(|m| m.id)
                    {
                        app.open_note(id);
                    }
                }
            }
        } else {
            app.startup_toast = app.open_paths(&files);
        }
        app.inbox = crate::ipc::Inbox::bind_default();
        app
    }

    fn with_library(look: Look, lib: Library) -> Self {
        let width = default_width(Nib::Fineliner);
        let dock_edge = lib.index.dock;
        let ink0 = look.inks.first().copied().unwrap_or(look.ink);
        Self {
            look,
            lib,
            scene: Scene::Shelf {
                query: String::new(),
            },
            note: None,
            page_delete: None,
            page_delete_view: None,
            tool: Tool::Fineliner,
            color_i: 0,
            ink: ink0,
            color_custom: false,
            tin_open: false,
            tin_hue: 0.0,
            width,
            last_ink: Tool::Fineliner,
            last_ink_width: width,
            last_eraser: Tool::EraserStroke,
            palm_grace_until: None,
            touch_grace_until: None,
            two_finger: None,
            camera: Camera::default(),
            undo: UndoStack::default(),
            live: None,
            shape_anchor: None,
            shape_still: None,
            shape_preview: None,
            lasso: Vec::new(),
            sel: Vec::new(),
            drag_last: None,
            editing_text: None,
            dirty: false,
            last_change: Instant::now(),
            textures: HashMap::new(),
            emoji: Atlas::load(),
            emoji_tex: HashMap::new(),
            pressure: Pressure::start(),
            last_ptr: None,
            toast: None,
            need_fit: true,
            fitted_cell: None,
            land_page: None,
            title_buf: String::new(),
            dock_edge,
            dock_float: None,
            dock_grab: Vec2::ZERO,
            dock_moved: false,
            canvas_rect: Rect::ZERO,
            tab_held: false,
            tablet: TabletBridge::new(),
            live_from_pen: false,
            cursor_off: false,
            pen_was_prox: false,
            emoji_pick: None,
            emoji_query: String::new(),
            rename_id: None,
            rename_buf: String::new(),
            rename_rect: None,
            text_focus: false,
            palette_open: false,
            pen_ui_grab: false,
            pending_image: None,
            pending_open: false,
            inbox: None,
            bg_textures: HashMap::new(),
            bg_order: VecDeque::new(),
            bg_bytes: 0,
            bg_failed: HashSet::new(),
            startup_toast: None,
            export_picker_open: false,
            shelf_sel: Vec::new(),
            shelf_anchor: None,
            shelf_trash: false,
            bin_panel_height: 0.0,
            bin_close_rect: Rect::NOTHING,
            shelf_slots: Vec::new(),
            shelf_band: None,
            shelf_band_now: None,
            shelf_band_add: false,
            shelf_band_armed: false,
            shelf_band_base: Vec::new(),
            shelf_haul: None,
            shelf_haul_now: None,
            shelf_haul_ids: Vec::new(),
            shelf_haul_armed: false,
            shelf_drop: None,
            shelf_grid: Vec::new(),
            trash_grid: Vec::new(),
            shelf_toss: Vec::new(),
            trash_rect: Rect::NOTHING,
            trash_mouth: Pos2::ZERO,
            trash_reveal: 0.0,
            shelf_fed: false,
            spine_wheel: None,
            shelf_hold_t0: None,
        }
    }

    fn toast(&mut self, msg: impl Into<String>, t: f64) {
        self.toast = Some(Toast {
            msg: msg.into(),
            until: t + 2.4,
        });
    }

    fn report_save(&mut self, result: Result<(), SaveError>) {
        self.lib.remember_save_error(result);
    }

    fn flush_save_error(&mut self, ctx: &Context) {
        if let Some(msg) = self.lib.take_save_error() {
            self.toast(format!("Couldn't save — {msg}"), ctx.input(|i| i.time));
        }
    }

    fn ink_color(&self) -> Color32 {
        if self.tool == Tool::Highlighter {
            Color32::from_rgba_unmultiplied(self.ink.r(), self.ink.g(), self.ink.b(), 90)
        } else {
            Color32::from_rgb(self.ink.r(), self.ink.g(), self.ink.b())
        }
    }

    fn pick_theme_ink(&mut self, i: usize) {
        self.color_custom = false;
        self.color_i = i;
        if self.tool == Tool::Highlighter {
            let n = self.look.highs.len().max(1);
            let c = self.look.highs[i % n];
            self.ink = Color32::from_rgb(c.r(), c.g(), c.b());
        } else {
            let n = self.look.inks.len().saturating_sub(1).max(1);
            self.ink = self.look.inks[i % n];
        }
        self.tin_hue = rgb_to_hsv(self.ink).0;
    }

    fn pick_free_ink(&mut self, c: Color32) {
        self.color_custom = true;
        self.ink = Color32::from_rgb(c.r(), c.g(), c.b());
        let (h, s, _) = rgb_to_hsv(self.ink);
        if s > 0.04 {
            self.tin_hue = h;
        }
    }

    fn ph(&self) -> f32 {
        self.note
            .as_ref()
            .map(|n| n.page_h.max(1.0))
            .unwrap_or(PAGE_H)
    }

    fn pw(&self) -> f32 {
        self.note
            .as_ref()
            .map(|n| n.page_w.max(1.0))
            .unwrap_or(PAGE_W)
    }

    fn page_cells(&self) -> Vec<(i32, i32)> {
        self.note
            .as_ref()
            .map(|n| n.pages.iter().map(|p| (p.col, p.row)).collect())
            .unwrap_or_else(|| vec![(0, 0)])
    }

    fn gap(&self) -> f32 {
        self.note
            .as_ref()
            .map(|n| n.sheet_join.gap())
            .unwrap_or(0.0)
    }

    fn origin_of(&self, i: usize) -> Vec2 {
        let cells = self.page_cells();
        let (col, row) = cells.get(i).copied().unwrap_or((0, 0));
        page_origin(col, row, self.pw(), self.ph(), self.gap())
    }

    fn page_at_paper(&self, paper: Pos2) -> usize {
        page_at(paper, &self.page_cells(), self.pw(), self.ph(), self.gap())
    }

    fn page_in_view(&self, rect: Rect) -> usize {
        self.page_at_paper(self.camera.to_paper(rect.center(), rect))
    }

    fn fit_zoom_now(&self) -> f32 {
        let rect = self.canvas_rect;
        if !Camera::valid_viewport(rect) {
            return 1.0;
        }
        let (pw, ph) = self
            .note
            .as_ref()
            .map(|n| n.page_size())
            .unwrap_or((PAGE_W, PAGE_H));
        Camera::fit_zoom(rect, pw, ph)
    }

    /// 100 = the sheet fills the screen.
    fn zoom_percent(&self) -> i32 {
        let fit = self.fit_zoom_now();
        ((self.camera.zoom / fit.max(0.001)) * 100.0).round() as i32
    }

    fn zoom_level(&self) -> f32 {
        self.camera.zoom / self.fit_zoom_now().max(0.001)
    }

    fn set_zoom_level(&mut self, level: f32) {
        let rect = self.canvas_rect;
        if !Camera::valid_viewport(rect) {
            return;
        }
        let target = (self.fit_zoom_now() * level).max(0.001);
        self.camera.set_zoom_at(rect.center(), rect, target);
        self.fitted_cell = None;
    }

    fn zoom_in(&mut self) {
        let cur = self.zoom_level();
        let next = ZOOM_STOPS
            .iter()
            .copied()
            .find(|&s| s > cur + 0.03)
            .unwrap_or(*ZOOM_STOPS.last().unwrap());
        self.set_zoom_level(next);
    }

    fn zoom_out(&mut self) {
        let cur = self.zoom_level();
        let next = ZOOM_STOPS
            .iter()
            .copied()
            .rev()
            .find(|&s| s < cur - 0.03)
            .unwrap_or(*ZOOM_STOPS.first().unwrap());
        self.set_zoom_level(next);
    }

    fn fit_to_screen(&mut self) {
        self.land_page = None;
        self.need_fit = true;
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.last_change = Instant::now();
        if let Some(n) = &mut self.note {
            n.touch();
        }
    }

    fn autosave(&mut self) {
        if !self.dirty {
            return;
        }
        if self.last_change.elapsed().as_millis() < 700 {
            return;
        }
        let Some(n) = &self.note else {
            return;
        };
        match self.lib.save_note(n) {
            Ok(()) => self.dirty = false,
            Err(err) => {
                self.last_change = Instant::now();
                self.report_save(Err(err));
            }
        }
    }

    fn open_note(&mut self, id: Uuid) {
        self.page_delete = None;
        self.page_delete_view = None;
        self.autosave();
        if let Some(n) = self.lib.load_note(id) {
            self.title_buf = n.title.clone();
            self.note = Some(n);
            self.scene = Scene::Desk;
            self.undo.clear();
            self.sel.clear();
            self.live = None;
            self.clear_shape_hold();
            self.lasso.clear();
            self.editing_text = None;
            self.need_fit = false;
            self.land_page = Some(0);
            self.fitted_cell = None;
            self.canvas_rect = Rect::ZERO;
            self.textures.clear();
            self.clear_backgrounds();
            self.tab_held = false;
        }
    }

    fn close_desk(&mut self) {
        self.page_delete = None;
        self.page_delete_view = None;
        self.autosave();
        if let Some(n) = self.note.take() {
            let saved = self.lib.save_note(&n);
            self.report_save(saved);
        }
        self.scene = Scene::Shelf {
            query: String::new(),
        };
        self.canvas_rect = Rect::ZERO;
        self.fitted_cell = None;
        self.undo.clear();
        self.sel.clear();
    }

    fn new_note(&mut self) {
        let cover = (self.lib.index.notes.len() as u8).wrapping_add(3);
        let n = Note::blank("Untitled", cover);
        let id = n.id;
        let saved = self.lib.insert_new(&n);
        self.report_save(saved);
        self.open_note(id);
    }

    fn save_now(&mut self, ctx: &Context) {
        let Some(n) = &self.note else {
            return;
        };
        match self.lib.save_note(n) {
            Ok(()) => {
                self.dirty = false;
                self.toast("Saved", ctx.input(|i| i.time));
            }
            Err(err) => self.report_save(Err(err)),
        }
    }

    fn save_for_download(&mut self, ctx: &Context) {
        self.save_now(ctx);
        self.export_picker_open = self.note.is_some();
    }

    fn set_sheet_join(&mut self, join: SheetJoin) {
        if self.note.as_ref().is_none_or(|n| n.sheet_join == join) {
            return;
        }
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.push_snapshot();
        let rect = self.canvas_rect;
        let focus = rect.center();
        let old_paper = if rect.width() > 10.0 {
            self.camera.to_paper(focus, rect)
        } else {
            pos2(0.0, 0.0)
        };
        let (cells, old_gap) = self
            .note
            .as_ref()
            .map(|n| (n.unit_cells(), n.sheet_join.gap()))
            .unwrap_or_else(|| (vec![(0, 0)], 0.0));
        let idx = page_at(old_paper, &cells, PAGE_W, PAGE_H, old_gap);
        let (col, row) = cells.get(idx).copied().unwrap_or((0, 0));
        let local = old_paper - page_origin(col, row, PAGE_W, PAGE_H, old_gap);
        if let Some(n) = &mut self.note {
            n.apply_sheet_join(join);
        }
        if rect.width() > 10.0 {
            let o = page_origin(col, row, PAGE_W, PAGE_H, join.gap());
            let new_world = pos2(o.x + local.x, o.y + local.y);
            let z = self.camera.zoom;
            self.camera.pan = (focus - rect.min) - vec2(new_world.x * z, new_world.y * z);
            self.fitted_cell = None;
        }
        self.sel.clear();
        self.lasso.clear();
        self.mark_dirty();
    }

    fn clear_backgrounds(&mut self) {
        self.bg_textures.clear();
        self.bg_order.clear();
        self.bg_bytes = 0;
        self.bg_failed.clear();
    }

    /// Import files. Images land on the open page; everything else becomes a notebook.
    fn open_paths(&mut self, paths: &[PathBuf]) -> Option<String> {
        let place_on_page = matches!(self.scene, Scene::Desk) && self.note.is_some();
        let mut notebooks = Vec::new();
        let mut placed = 0usize;
        let mut messages = Vec::new();
        for (i, path) in paths.iter().enumerate() {
            if place_on_page && crate::import::is_placeable_image(path) {
                match crate::import::load_placed_image(path) {
                    Ok(png) => {
                        let (page, local) = self.drop_spot();
                        let local = local + vec2(i as f32 * 28.0, i as f32 * 28.0);
                        self.insert_image_bytes(&png, page, local);
                        placed += 1;
                    }
                    Err(err) => messages.push(err.to_string()),
                }
            } else {
                match crate::import::import_new_note(&mut self.lib, path) {
                    Ok(note) => notebooks.push((note.id, note.title)),
                    Err(err) => messages.push(err.to_string()),
                }
            }
        }
        if let Some((id, _)) = notebooks.first() {
            self.open_note(*id);
        }
        let mut parts = Vec::new();
        match notebooks.as_slice() {
            [] => {}
            [(_, title)] if placed == 0 => parts.push(format!("Opened {title}")),
            many => parts.push(format!("Opened {} files", many.len())),
        }
        if placed == 1 {
            parts.push("Placed an image".into());
        } else if placed > 1 {
            parts.push(format!("Placed {placed} images"));
        }
        if let Some(msg) = messages.first() {
            parts.push(msg.clone());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" · "))
        }
    }

    fn pick_open_files(&mut self, ctx: &Context) {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Open")
            .add_filter(
                "Notes",
                &[
                    "pdf", "png", "jpg", "jpeg", "webp", "svg", "txt", "md", "markdown",
                ],
            )
            .pick_files()
        else {
            return;
        };
        if let Some(msg) = self.open_paths(&paths) {
            self.toast(msg, ctx.input(|i| i.time));
        }
    }

    fn trash_open_note(&mut self) {
        self.autosave();
        if let Some(n) = self.note.take() {
            let saved = self.lib.save_note(&n);
            self.report_save(saved);
            let trashed = self.lib.trash_note(n.id);
            self.report_save(trashed);
        }
        self.scene = Scene::Shelf {
            query: String::new(),
        };
        self.undo.clear();
        self.sel.clear();
    }

    fn palm_guard(&self, pen: &PenSnapshot) -> bool {
        pen.in_proximity
            || self
                .palm_grace_until
                .map(|t| Instant::now() < t)
                .unwrap_or(false)
    }

    fn finger_alive(&self, ui: &Ui) -> bool {
        ui.input(|i| i.any_touches())
            || self
                .touch_grace_until
                .map(|t| Instant::now() < t)
                .unwrap_or(false)
    }

    fn note_touches(&mut self, ui: &Ui) {
        let hit = ui.input(|i| {
            i.any_touches() || i.events.iter().any(|e| matches!(e, Event::Touch { .. }))
        });
        if hit {
            self.touch_grace_until = Some(Instant::now() + Duration::from_millis(120));
        }
    }

    fn remember_ink(&mut self) {
        if self.tool.is_ink() {
            self.last_ink = self.tool;
            self.last_ink_width = self.width;
        }
    }

    fn toggle_eraser(&mut self) {
        if self.tool.is_eraser() {
            self.tool = self.last_ink;
            self.width = self.last_ink_width;
        } else {
            self.remember_ink();
            self.tool = self.last_eraser;
        }
    }

    /// Picks an eraser; tapping the same one again returns to ink.
    fn pick_eraser(&mut self, kind: Tool) {
        if !kind.is_eraser() {
            return;
        }
        if self.tool == kind {
            self.tool = self.last_ink;
            self.width = self.last_ink_width;
        } else {
            self.remember_ink();
            self.last_eraser = kind;
            self.tool = kind;
        }
    }

    fn is_tablette(&self) -> bool {
        self.palm_guard(&self.tablet.snapshot())
    }

    fn slot(&self) -> f32 {
        44.0
    }

    fn chrome_btn(&self) -> f32 {
        44.0
    }

    fn drop_spot(&self) -> (usize, Pos2) {
        let rect = self.canvas_rect;
        let paper = if rect.width() > 10.0 {
            self.camera.to_paper(rect.center(), rect)
        } else {
            pos2(80.0, 80.0)
        };
        let page = self.page_at_paper(paper);
        let local = paper - self.origin_of(page);
        (page, local)
    }

    fn finish_text_edit(&mut self) {
        let Some((pg, id)) = self.editing_text.take() else {
            return;
        };
        self.text_focus = false;
        let empty = self
            .note
            .as_ref()
            .and_then(|n| n.pages.get(pg))
            .and_then(|p| p.texts.iter().find(|t| t.id == id))
            .map(|t| t.text.trim().is_empty())
            .unwrap_or(false);
        if empty {
            if let Some(n) = &mut self.note {
                if let Some(page) = n.pages.get_mut(pg) {
                    page.texts.retain(|t| t.id != id);
                }
            }
            self.mark_dirty();
        }
    }
}

impl eframe::App for CahierApp {
    fn raw_input_hook(&mut self, ctx: &Context, raw_input: &mut egui::RawInput) {
        if self.tablet.ready() {
            self.tablet.pump_events();
            self.inject_pen_pointer(ctx, raw_input);
        }
    }

    fn update(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        self.tablet.ensure(frame);
        let pen = self.tablet.snapshot();
        if let Some(p) = pen.pressure {
            self.pressure.push_touch(p);
        } else if !pen.in_proximity {
            self.pressure.clear();
        }
        if pen.in_proximity {
            self.palm_grace_until = Some(Instant::now() + Duration::from_millis(180));
        }
        if matches!(self.scene, Scene::Desk) && pen.air_toggle {
            self.toggle_eraser();
        }
        if self.look.drifted() {
            let next = Look::load();
            if next.stamp != self.look.stamp {
                let label = next.name.clone();
                self.look = next;
                if !self.color_custom {
                    self.pick_theme_ink(self.color_i);
                }
                self.look.apply(ctx);
                crate::fonts::install(ctx);
                let t = ctx.input(|i| i.time);
                self.toast(format!("Theme · {label}"), t);
            }
        }
        let wants_pen = self.tablet.wants_repaint();
        let t = ctx.input(|i| i.time);
        let incoming = self
            .inbox
            .as_ref()
            .map(|inbox| inbox.poll())
            .unwrap_or_default();
        if !incoming.is_empty() {
            if let Some(msg) = self.open_paths(&incoming) {
                self.toast(msg, t);
            }
        }
        if let Some(msg) = self.startup_toast.take() {
            self.toast(msg, t);
        }
        self.shortcuts(ctx);
        self.ingest_touch_pressure(ctx);

        match self.scene.clone() {
            Scene::Shelf { .. } => self.ui_shelf(ctx),
            Scene::Desk => self.ui_desk(ctx),
        }

        if self.pending_open {
            self.pending_open = false;
            self.pick_open_files(ctx);
        }
        self.autosave();
        self.flush_save_error(ctx);
        if let Some(toast) = &self.toast {
            if t < toast.until {
                let msg = toast.msg.clone();
                let paper = self.paper_tex(ctx);
                ShowToast { look: &self.look }.show(ctx, &msg, &paper);
            } else {
                self.toast = None;
            }
        }

        self.sync_pen_cursor(ctx);

        let focused = ctx.input(|i| i.focused);
        let busy = self.live.is_some()
            || !self.lasso.is_empty()
            || self.dirty
            || !self.sel.is_empty()
            || self.dock_float.is_some()
            || wants_pen
            || self.toast.is_some()
            || pen.in_proximity
            || pen.down;
        if busy && focused {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let saved = if let Some(n) = &self.note {
            self.lib.save_note(n)
        } else {
            self.lib.save_index()
        };
        if let Err(err) = saved {
            eprintln!("cahier: {err}");
        }
    }
}

struct ShowToast<'a> {
    look: &'a Look,
}

impl ShowToast<'_> {
    fn show(&self, ctx: &Context, msg: &str, paper: &TextureHandle) {
        Area::new(Id::new("toast"))
            .anchor(Align2::CENTER_TOP, vec2(0.0, 56.0))
            .show(ctx, |ui| {
                let mut grain = None;
                let card = Frame::NONE
                    .fill(self.look.paper)
                    .corner_radius(8)
                    .stroke(Stroke::new(1.0_f32, self.look.ink.gamma_multiply(0.18)))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 4],
                        blur: 8,
                        spread: 0,
                        color: self.look.shadow.gamma_multiply(0.42),
                    })
                    .inner_margin(Margin::symmetric(16, 8))
                    .show(ui, |ui| {
                        grain = Some(ui.painter().add(Shape::Noop));
                        ui.label(
                            RichText::new(msg)
                                .font(self.look.mono(13.0))
                                .color(self.look.ink),
                        );
                    });
                if let Some(grain) = grain {
                    ui.painter()
                        .set(grain, paper_grain(card.response.rect.shrink(1.0), 7, paper));
                }
            });
    }
}

impl CahierApp {
    fn ingest_touch_pressure(&self, ctx: &Context) {
        ctx.input(|i| {
            for ev in &i.events {
                if let Event::Touch { force: Some(f), .. } = ev {
                    self.pressure.push_touch(*f);
                }
            }
        });
    }

    fn shortcuts(&mut self, ctx: &Context) {
        let mut new_note = false;
        let mut open_file = false;
        let mut undo = false;
        let mut redo = false;
        let mut save = false;
        let mut export_png = false;
        let mut export_pdf = false;
        let mut fit = false;
        let mut zoom_in = false;
        let mut zoom_out = false;
        let mut close = false;
        let mut delete_sel = false;
        let mut remove_page = false;
        let mut dup = false;
        let mut width_delta: f32 = 0.0;
        let mut tool: Option<Tool> = None;
        let mut color: Option<usize> = None;
        let mut cycle_paper = false;
        let mut erase_pick: Option<Tool> = None;
        let mut toggle_fiche = false;

        let typing = ctx.wants_keyboard_input()
            || self.editing_text.is_some()
            || self.rename_id.is_some()
            || self.emoji_pick.is_some();
        ctx.input(|i| {
            let c = i.modifiers.command;
            let sh = i.modifiers.shift;
            if i.key_pressed(Key::Escape) {
                close = true;
            }
            if c && i.key_pressed(Key::N) {
                new_note = true;
            }
            if c && i.key_pressed(Key::O) {
                open_file = true;
            }
            if c && i.key_pressed(Key::Z) && !sh {
                undo = true;
            }
            if c && (i.key_pressed(Key::Y) || (sh && i.key_pressed(Key::Z))) {
                redo = true;
            }
            if c && i.key_pressed(Key::S) {
                save = true;
            }
            if c && sh && i.key_pressed(Key::E) {
                export_pdf = true;
            } else if c && i.key_pressed(Key::E) {
                export_png = true;
            }
            if (c && i.key_pressed(Key::Num0)) || (!typing && !c && i.key_pressed(Key::Num0)) {
                fit = true;
            }
            if !typing
                && (i.key_pressed(Key::Plus)
                    || i.key_pressed(Key::Equals)
                    || (c && i.key_pressed(Key::Equals)))
            {
                zoom_in = true;
            }
            if !typing && i.key_pressed(Key::Minus) {
                zoom_out = true;
            }
            if c && i.key_pressed(Key::D) {
                dup = true;
            }
            if !typing && c && sh && i.key_pressed(Key::Delete) {
                remove_page = true;
            } else if (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace))
                && self.editing_text.is_none()
            {
                delete_sel = true;
            }
            if i.key_pressed(Key::OpenBracket) {
                width_delta = -0.8;
            }
            if i.key_pressed(Key::CloseBracket) {
                width_delta = 0.8;
            }
            if !c && self.editing_text.is_none() {
                if i.key_pressed(Key::P) {
                    tool = Some(Tool::Fineliner);
                }
                if i.key_pressed(Key::B) {
                    tool = Some(Tool::Brush);
                }
                if i.key_pressed(Key::C) {
                    tool = Some(Tool::Pencil);
                }
                if i.key_pressed(Key::H) {
                    tool = Some(Tool::Highlighter);
                }
                if i.key_pressed(Key::E) {
                    erase_pick = Some(if sh {
                        Tool::EraserArea
                    } else {
                        Tool::EraserStroke
                    });
                }
                if i.key_pressed(Key::L) {
                    tool = Some(Tool::Lasso);
                }
                if i.key_pressed(Key::T) {
                    tool = Some(Tool::Text);
                }
                if i.key_pressed(Key::I) {
                    tool = Some(Tool::Image);
                }
                if i.key_pressed(Key::M) {
                    cycle_paper = true;
                }
                for (k, idx) in [
                    (Key::Num1, 0),
                    (Key::Num2, 1),
                    (Key::Num3, 2),
                    (Key::Num4, 3),
                    (Key::Num5, 4),
                    (Key::Num6, 5),
                    (Key::Num7, 6),
                    (Key::Num8, 7),
                    (Key::Num9, 8),
                ] {
                    if i.key_pressed(k) {
                        color = Some(idx);
                    }
                }
            }
            if !c {
                if let Scene::Shelf { .. } = self.scene {
                    if !typing && i.key_pressed(Key::N) {
                        new_note = true;
                    }
                    if !typing && (i.key_pressed(Key::Slash) || i.key_pressed(Key::F1)) {
                        toggle_fiche = true;
                    }
                }
            }
        });

        if new_note {
            self.new_note();
        }
        if open_file {
            self.pending_open = true;
        }
        if close {
            match &self.scene {
                Scene::Desk => {
                    if Popup::is_any_open(ctx) {
                        Popup::close_all(ctx);
                        ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                    } else if self.export_picker_open {
                        self.export_picker_open = false;
                    } else if self.tin_open {
                        self.tin_open = false;
                    } else if self.palette_open {
                        self.palette_open = false;
                    } else if self.page_delete.is_some() {
                        self.cancel_page_delete();
                    } else if self.editing_text.is_some() {
                        self.finish_text_edit();
                    } else if !self.sel.is_empty() {
                        self.sel.clear();
                    } else {
                        self.close_desk();
                    }
                }
                Scene::Shelf { .. } => {
                    // Shelf navigation is handled once in shelf_keys; modals take priority.
                    if self.emoji_pick.is_some() {
                        self.emoji_pick = None;
                        self.emoji_query.clear();
                        ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                    }
                }
            }
        }
        if let Scene::Desk = self.scene {
            if remove_page {
                self.start_page_delete();
            }
            if undo {
                if let Some(n) = &mut self.note {
                    if self.undo.undo(n) {
                        self.mark_dirty();
                    }
                }
            }
            if redo {
                if let Some(n) = &mut self.note {
                    if self.undo.redo(n) {
                        self.mark_dirty();
                    }
                }
            }
            if save {
                self.save_for_download(ctx);
            }
            if export_png {
                self.export_png(ctx);
            }
            if export_pdf {
                self.export_pdf(ctx);
            }
            if fit {
                self.fit_to_screen();
            }
            if zoom_in {
                self.zoom_in();
            }
            if zoom_out {
                self.zoom_out();
            }
            if delete_sel {
                self.delete_selection();
            }
            if dup {
                self.duplicate_selection();
            }
            if width_delta != 0.0 {
                self.width = (self.width + width_delta).clamp(0.8, 48.0);
                if self.tool.is_ink() {
                    self.last_ink_width = self.width;
                }
            }
            if let Some(kind) = erase_pick {
                self.pick_eraser(kind);
            } else if let Some(t) = tool {
                if let Some(nib) = t.nib() {
                    self.last_ink = t;
                    let width = default_width(nib);
                    self.last_ink_width = width;
                    self.width = width;
                }
                if t.is_eraser() {
                    self.remember_ink();
                    self.last_eraser = t;
                }
                self.tool = t;
                if t == Tool::Image {
                    self.pending_image = Some(self.drop_spot());
                }
            }
            if let Some(i) = color {
                self.pick_theme_ink(i);
            }
            if cycle_paper {
                if let Some(n) = &mut self.note {
                    n.paper = n.paper.cycle();
                    self.mark_dirty();
                }
            }
        }
        if toggle_fiche {
            self.lib.index.fiche_pliee = !self.lib.index.fiche_pliee;
            let saved = self.lib.save_index();
            self.report_save(saved);
        }
    }

    fn push_snapshot(&mut self) {
        if let Some(n) = &self.note {
            self.undo.push(n);
        }
    }

    fn delete_selection(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.push_snapshot();
        let sel = self.sel.clone();
        if let Some(n) = &mut self.note {
            for s in sel {
                if let Some(page) = n.pages.get_mut(s.page) {
                    match s.kind {
                        SelKind::Stroke => page.strokes.retain(|x| x.id != s.id),
                        SelKind::Text => page.texts.retain(|x| x.id != s.id),
                        SelKind::Image => page.images.retain(|x| x.id != s.id),
                    }
                }
            }
        }
        self.sel.clear();
        self.mark_dirty();
    }

    fn duplicate_selection(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.push_snapshot();
        let sel = self.sel.clone();
        let mut neu = Vec::new();
        if let Some(n) = &mut self.note {
            for s in sel {
                let Some(page) = n.pages.get_mut(s.page) else {
                    continue;
                };
                match s.kind {
                    SelKind::Stroke => {
                        if let Some(st) = page.strokes.iter().find(|x| x.id == s.id).cloned() {
                            let mut st = st;
                            st.id = Uuid::new_v4();
                            st.translate(vec2(16.0, 16.0));
                            neu.push(Sel {
                                page: s.page,
                                id: st.id,
                                kind: SelKind::Stroke,
                            });
                            page.strokes.push(st);
                        }
                    }
                    SelKind::Text => {
                        if let Some(mut tx) = page.texts.iter().find(|x| x.id == s.id).cloned() {
                            tx.id = Uuid::new_v4();
                            tx.translate(vec2(16.0, 16.0));
                            neu.push(Sel {
                                page: s.page,
                                id: tx.id,
                                kind: SelKind::Text,
                            });
                            page.texts.push(tx);
                        }
                    }
                    SelKind::Image => {
                        if let Some(mut im) = page.images.iter().find(|x| x.id == s.id).cloned() {
                            im.id = Uuid::new_v4();
                            im.translate(vec2(16.0, 16.0));
                            neu.push(Sel {
                                page: s.page,
                                id: im.id,
                                kind: SelKind::Image,
                            });
                            page.images.push(im);
                        }
                    }
                }
            }
        }
        self.sel = neu;
        self.mark_dirty();
    }

    fn ui_shelf(&mut self, ctx: &Context) {
        self.handle_drops_and_paste(ctx);
        // Shelf shortcuts (selection / trash)
        if self.emoji_pick.is_none() && self.rename_id.is_none() && self.lib.index.fiche_pliee {
            self.shelf_keys(ctx);
            self.spine_wheel_tick(ctx);
            self.shelf_pointer_tick(ctx);
            self.shelf_toss_tick(ctx);
            if self.shelf_haul_armed
                || !self.shelf_toss.is_empty()
                || self.spine_wheel.is_some()
                || self.shelf_hold_t0.is_some()
            {
                ctx.request_repaint();
            }
        }

        let query = match &self.scene {
            Scene::Shelf { query } => query.to_lowercase(),
            _ => String::new(),
        };
        let querying = !query.is_empty();
        let mut open = None;
        let mut select = None;
        let mut toggle = None;
        let mut range = None;
        self.shelf_slots.clear();
        self.shelf_grid.clear();
        self.trash_grid.clear();

        self.ui_bin_zone(ctx, &mut open, &mut select, &mut toggle, &mut range);
        self.paint_trash_drop_zone(ctx);

        CentralPanel::default()
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                let notes: Vec<_> = self
                    .lib
                    .index
                    .notes
                    .iter()
                    .filter(|m| query.is_empty() || m.title.to_lowercase().contains(&query))
                    .filter(|m| !self.shelf_toss.iter().any(|t| t.id == m.id))
                    .cloned()
                    .collect();

                ui.add_space(if ctx.screen_rect().width() < 700.0 {
                    124.0
                } else {
                    92.0
                });

                let shifting = self.shelf_haul_armed && !querying;
                let packed = querying;
                let shelf_height =
                    (ui.available_height() - if self.shelf_trash { 0.0 } else { 88.0 }).max(1.0);

                ScrollArea::vertical()
                    .id_salt("shelf-grid")
                    .max_height(shelf_height)
                    .scroll_source(ScrollSource {
                        drag: false,
                        scroll_bar: true,
                        mouse_wheel: true,
                    })
                    .show(ui, |ui| {
                        if let Some(mt) = ui.input(|i| i.multi_touch()) {
                            if mt.num_touches >= 2 && mt.translation_delta != Vec2::ZERO {
                                ui.scroll_with_delta_animation(
                                    mt.translation_delta,
                                    ScrollAnimation::none(),
                                );
                                ui.ctx().request_repaint();
                            }
                        }
                        ui.add_space(4.0);
                        let left = if ui.available_width() < 400.0 {
                            8.0
                        } else {
                            28.0
                        };
                        let compact_cards = ctx.screen_rect().height() < 520.0;
                        let slot = if compact_cards {
                            vec2(216.0, (shelf_height - 4.0).clamp(44.0, 64.0))
                        } else {
                            Self::shelf_slot()
                        };
                        let pitch = vec2(slot.x + SHELF_GAP, slot.y + SHELF_GAP);
                        let cols = (((ui.available_width() - left) / pitch.x).floor().max(1.0)
                            as usize)
                            .min(SHELF_COLS as usize);
                        let count = if packed {
                            notes.len().max(1)
                        } else {
                            SHELF_SLOTS as usize
                        };
                        let rows = count.div_ceil(cols);
                        let n_cells = if packed {
                            rows * cols
                        } else {
                            SHELF_SLOTS as usize
                        };
                        let grid = vec2(left + cols as f32 * pitch.x, rows as f32 * pitch.y);
                        let (full, _) = ui.allocate_exact_size(grid, Sense::hover());
                        let origin = pos2(full.min.x + left, full.min.y);
                        let occ: HashMap<u32, crate::library::NoteMeta> = if packed {
                            HashMap::new()
                        } else {
                            notes.iter().map(|m| (m.slot, m.clone())).collect()
                        };
                        for idx in 0..n_cells {
                            let col = idx % cols;
                            let row = idx / cols;
                            let cell = Rect::from_min_size(
                                origin + vec2(col as f32 * pitch.x, row as f32 * pitch.y),
                                slot,
                            );
                            self.shelf_grid.push(cell.intersect(ui.clip_rect()));
                            let meta = if packed {
                                notes.get(idx).cloned()
                            } else {
                                occ.get(&(idx as u32)).cloned()
                            };
                            let lifted = meta
                                .as_ref()
                                .map(|m| shifting && self.shelf_haul_ids.contains(&m.id))
                                .unwrap_or(false);
                            let hot = shifting && self.shelf_drop == Some(ShelfDrop::Shelf(idx));
                            if let Some(meta) = meta.filter(|_| !lifted) {
                                ui.scope_builder(UiBuilder::new().max_rect(cell), |ui| {
                                    let selected = self.shelf_sel.contains(&meta.id);
                                    let action = if compact_cards {
                                        self.compact_dos(ui, &meta, selected, false, slot)
                                    } else {
                                        self.cahier_dos(ui, &meta, selected, false)
                                    };
                                    match action {
                                        Some(DosAct::Open) => open = Some(meta.id),
                                        Some(DosAct::Select) => select = Some(meta.id),
                                        Some(DosAct::Toggle) => toggle = Some(meta.id),
                                        Some(DosAct::Range) => range = Some(meta.id),
                                        None => {}
                                    }
                                });
                            } else if shifting {
                                if compact_cards {
                                    self.paint_sel_wash(
                                        &ui.painter_at(cell),
                                        cell.shrink(3.0),
                                        hot,
                                    );
                                } else {
                                    self.place_dos(ui, cell, hot);
                                }
                            }
                        }
                    });
            });

        let ids: Vec<Uuid> = {
            let mut v = self.lib.index.notes.clone();
            v.sort_by_key(|n| n.slot);
            let mut ids: Vec<_> = v.into_iter().map(|n| n.id).collect();
            if self.shelf_trash {
                let mut t = self.lib.index.trash.clone();
                t.sort_by_key(|n| n.slot);
                ids.extend(t.into_iter().map(|n| n.id));
            }
            ids
        };
        if !self.shelf_band_armed {
            if let Some(id) = select {
                self.shelf_sel = vec![id];
                self.shelf_anchor = Some(id);
            }
            if let Some(id) = toggle {
                if let Some(p) = self.shelf_sel.iter().position(|x| *x == id) {
                    self.shelf_sel.remove(p);
                } else {
                    self.shelf_sel.push(id);
                }
                self.shelf_anchor = Some(id);
            }
            if let Some(id) = range {
                let anchor = self.shelf_anchor.or_else(|| self.shelf_sel.last().copied());
                if let Some(a) = anchor {
                    if let (Some(ia), Some(ib)) = (
                        ids.iter().position(|x| *x == a),
                        ids.iter().position(|x| *x == id),
                    ) {
                        let (lo, hi) = if ia <= ib { (ia, ib) } else { (ib, ia) };
                        self.shelf_sel = ids[lo..=hi].to_vec();
                    }
                } else {
                    self.shelf_sel = vec![id];
                    self.shelf_anchor = Some(id);
                }
            }
        }
        if let Some(id) = open {
            if !self.lib.is_trashed(id) && !self.shelf_band_armed {
                self.emoji_pick = None;
                self.rename_id = None;
                self.shelf_sel.clear();
                self.open_note(id);
            }
        }

        if let (Some(a), Some(b)) = (self.shelf_band, self.shelf_band_now) {
            if self.shelf_band_armed {
                Area::new(Id::new("shelf-band"))
                    .fixed_pos(ctx.screen_rect().min)
                    .order(Order::Foreground)
                    .interactable(false)
                    .show(ctx, |ui| {
                        let r = Rect::from_two_pos(a, b);
                        let p = ui.painter();
                        let fill = self.look.accent.gamma_multiply(0.16);
                        let stroke = Stroke::new(1.15_f32, self.look.accent.gamma_multiply(0.85));
                        p.rect_filled(r, CornerRadius::same(2), fill);
                        p.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Inside);
                    });
            }
        }

        // Keep the drag target represented by the large icon in the bin panel.
        if !self.shelf_trash && self.bin_panel_height == 0.0 {
            self.wastebasket(ctx);
        }

        Area::new(Id::new("shelf-top-left"))
            .anchor(Align2::LEFT_TOP, vec2(16.0, 10.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                let open = self.open_signet(ui);
                open.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Open file")
                });
                if open.clicked() {
                    self.pending_open = true;
                }
            });

        Area::new(Id::new("shelf-search"))
            .anchor(
                Align2::CENTER_TOP,
                vec2(
                    0.0,
                    if ctx.screen_rect().width() < 700.0 {
                        76.0
                    } else {
                        16.0
                    },
                ),
            )
            .order(Order::Foreground)
            .show(ctx, |ui| {
                if let Scene::Shelf { query } = &mut self.scene {
                    let width = ctx.screen_rect().width();
                    let search_w = if width < 700.0 {
                        (width - 68.0).max(1.0)
                    } else {
                        (width - 476.0).clamp(1.0, 560.0)
                    };
                    Frame::NONE
                        .fill(self.look.desk_deep)
                        .corner_radius(22)
                        .inner_margin(Margin::symmetric(18, 11))
                        .show(ui, |ui| {
                            ui.set_width(search_w);
                            let te = TextEdit::singleline(query)
                                .hint_text("Search")
                                .font(self.look.mono(15.0))
                                .frame(false)
                                .id(Id::new("shelf-search-field"));
                            ui.add(te);
                        });
                }
            });

        let fiche_ouverte = !self.lib.index.fiche_pliee;
        Area::new(Id::new("shelf-top-right"))
            .anchor(Align2::RIGHT_TOP, vec2(-16.0, 10.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                    if self.help_signet(ui, fiche_ouverte).clicked() {
                        self.lib.index.fiche_pliee = !self.lib.index.fiche_pliee;
                        let saved = self.lib.save_index();
                        self.report_save(saved);
                    }
                    if self.inkwell(ui).clicked() {
                        self.new_note();
                    }
                    if self.quit_signet(ui).clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                });
            });

        if fiche_ouverte {
            let response = Modal::new(Id::new("shelf-tuto-fiche"))
                .frame(Frame::NONE.fill(self.look.paper).corner_radius(8))
                .show(ctx, |ui| {
                    ui.set_width(560.0_f32.min(ctx.screen_rect().width() - 32.0));
                    ScrollArea::vertical()
                        .max_height((ctx.screen_rect().height() - 32.0).max(44.0))
                        .show(ui, |ui| self.fiche_pupitre(ui));
                });
            if response.should_close() {
                self.lib.index.fiche_pliee = true;
                let saved = self.lib.save_index();
                self.report_save(saved);
            }
        }
        self.paint_shelf_haul(ctx);
        self.paint_spine_wheel(ctx);
        self.shelf_rename_field(ctx);
        if let Some(id) = self.emoji_pick {
            self.ui_emoji_picker(ctx, id);
        }
        if ctx.input(|i| i.pointer.primary_released()) {
            self.shelf_band = None;
            self.shelf_band_now = None;
            self.shelf_band_armed = false;
        }
        self.shelf_fed = false;
    }

    fn shelf_keys(&mut self, ctx: &Context) {
        let typing =
            ctx.wants_keyboard_input() || self.rename_id.is_some() || self.emoji_pick.is_some();
        let mut clear = false;
        let mut trash_sel = false;
        let mut select_all = false;
        let mut open_one = false;
        let mut pin = false;
        let mut mark = false;
        let mut restore = false;
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                clear = true;
            }
            if !typing && i.modifiers.command && i.key_pressed(Key::A) {
                select_all = true;
            }
            if !typing && (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
                trash_sel = true;
            }
            if !typing && i.key_pressed(Key::Enter) && self.shelf_sel.len() == 1 {
                open_one = true;
            }
            if !typing && !i.modifiers.command && i.key_pressed(Key::P) {
                pin = true;
            }
            if !typing && !i.modifiers.command && i.key_pressed(Key::E) {
                mark = true;
            }
            if !typing && !i.modifiers.command && i.key_pressed(Key::R) {
                restore = true;
            }
        });
        if clear {
            if self.spine_wheel.is_some() {
                self.spine_wheel = None;
            } else if self.shelf_trash {
                self.close_bin(ctx);
            } else if !self.shelf_sel.is_empty() {
                self.shelf_sel.clear();
                self.shelf_anchor = None;
            }
            return;
        }
        if select_all {
            self.shelf_sel = self.lib.index.notes.iter().map(|m| m.id).collect();
        }
        let ids = self.shelf_action_ids(ctx);
        if trash_sel && !ids.is_empty() {
            for id in &ids {
                if self.lib.is_trashed(*id) {
                    let saved = self.lib.purge_trashed(*id);
                    self.report_save(saved);
                } else {
                    let saved = self.lib.trash_note(*id);
                    self.report_save(saved);
                }
            }
            self.shelf_sel.retain(|id| !ids.contains(id));
        }
        if pin && !ids.is_empty() {
            let pin_on = ids.iter().any(|id| {
                self.lib
                    .index
                    .notes
                    .iter()
                    .find(|m| m.id == *id)
                    .is_some_and(|m| !m.pinned)
            });
            for id in &ids {
                if self.lib.is_trashed(*id) {
                    continue;
                }
                if let Some(mut note) = self.lib.load_note(*id) {
                    note.pinned = pin_on;
                    let saved = self.lib.save_note(&note);
                    self.report_save(saved);
                    if pin_on {
                        let saved = self.lib.bring_front(*id);
                        self.report_save(saved);
                    }
                }
            }
        }
        if mark {
            if let Some(id) = ids.first().copied() {
                if !self.lib.is_trashed(id) {
                    self.emoji_pick = Some(id);
                    self.rename_id = None;
                }
            }
        }
        if restore {
            for id in &ids {
                if self.lib.is_trashed(*id) {
                    let saved = self.lib.restore_note(*id);
                    self.report_save(saved);
                }
            }
            self.shelf_sel.retain(|id| !ids.contains(id));
        }
        if open_one {
            if let Some(id) = self.shelf_sel.first().copied() {
                if !self.lib.is_trashed(id) {
                    self.shelf_sel.clear();
                    self.open_note(id);
                }
            }
        }
    }

    fn shelf_action_ids(&self, ctx: &Context) -> Vec<Uuid> {
        if !self.shelf_sel.is_empty() {
            return self.shelf_sel.clone();
        }
        let Some(pos) = ctx.pointer_hover_pos() else {
            return Vec::new();
        };
        self.shelf_slots
            .iter()
            .find(|(_, r)| r.contains(pos))
            .map(|(id, _)| vec![*id])
            .unwrap_or_default()
    }

    fn shelf_pointer_tick(&mut self, ctx: &Context) {
        let typing = ctx.wants_keyboard_input();
        let (pressed, down, released, pos, add, sec_pressed, touching, now) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
                i.modifiers.command || i.modifiers.shift,
                i.pointer.button_pressed(PointerButton::Secondary),
                i.any_touches(),
                i.time,
            )
        });
        let Some(pos) = pos else {
            if released {
                self.shelf_band = None;
                self.shelf_band_now = None;
                self.shelf_band_armed = false;
                self.shelf_haul = None;
                self.shelf_haul_now = None;
                self.shelf_haul_armed = false;
                self.shelf_haul_ids.clear();
                self.shelf_drop = None;
            }
            return;
        };
        let screen = ctx.screen_rect();
        let chrome = pos.y < screen.min.y + if screen.width() < 700.0 { 124.0 } else { 76.0 }
            || pos.x < screen.min.x + 12.0;
        let on_trash = self.over_trash_drop_zone(pos, screen, 12.0);
        let querying = matches!(&self.scene, Scene::Shelf { query } if !query.trim().is_empty());
        let hit = self
            .shelf_slots
            .iter()
            .find(|(_, r)| r.contains(pos))
            .map(|(id, _)| *id);

        if ctx.input(|i| i.multi_touch().is_some_and(|mt| mt.num_touches >= 2)) {
            self.spine_wheel = None;
            self.shelf_haul = None;
            self.shelf_haul_now = None;
            self.shelf_haul_armed = false;
            self.shelf_haul_ids.clear();
            self.shelf_drop = None;
            self.shelf_band = None;
            self.shelf_band_now = None;
            self.shelf_band_armed = false;
            self.shelf_hold_t0 = None;
            return;
        }

        if self.spine_wheel.is_some() {
            if released {
                self.shelf_haul = None;
                self.shelf_haul_now = None;
                self.shelf_haul_armed = false;
                self.shelf_haul_ids.clear();
                self.shelf_drop = None;
            }
            return;
        }

        // The drawer actions own their press, never a notebook drag or a selection band.
        if (pressed && self.bin_close_rect.contains(pos) && self.shelf_haul.is_none())
            || (!self.shelf_trash
                && self.bin_panel_height > 0.0
                && pos.y >= screen.bottom() - self.bin_panel_height)
        {
            self.shelf_fed = true;
            return;
        }

        if sec_pressed && !typing && !on_trash && !self.shelf_fed {
            if let Some(id) = hit {
                if !self.lib.is_trashed(id) {
                    self.open_spine_wheel(id, pos, ctx, true);
                    return;
                }
            }
        }

        if pressed && !typing && !on_trash && !self.shelf_fed {
            if let Some(id) = hit {
                if !add {
                    self.shelf_haul = Some(pos);
                    self.shelf_haul_now = Some(pos);
                    self.shelf_haul_armed = false;
                    self.shelf_drop = None;
                    // Hold is for a finger; the mouse opens the wheel with right-click.
                    if touching && !self.lib.is_trashed(id) {
                        self.shelf_hold_t0 = Some(now);
                    }
                    let from_bin = self.lib.is_trashed(id);
                    self.shelf_haul_ids = if self.shelf_sel.contains(&id) {
                        self.shelf_sel
                            .iter()
                            .copied()
                            .filter(|x| self.lib.is_trashed(*x) == from_bin)
                            .collect()
                    } else {
                        vec![id]
                    };
                } else {
                    self.shelf_band = Some(pos);
                    self.shelf_band_now = Some(pos);
                    self.shelf_band_add = add;
                    self.shelf_band_armed = false;
                    self.shelf_band_base = self.shelf_sel.clone();
                }
            } else if !chrome && pos.y < screen.max.y - 100.0 {
                self.shelf_band = Some(pos);
                self.shelf_band_now = Some(pos);
                self.shelf_band_add = add;
                self.shelf_band_armed = false;
                self.shelf_band_base = self.shelf_sel.clone();
            }
        }
        if down {
            if let Some(origin) = self.shelf_haul {
                self.shelf_haul_now = Some(pos);
                if origin.distance(pos) > 12.0 {
                    self.shelf_haul_armed = true;
                    self.shelf_hold_t0 = None;
                    if let Some(id) = self.shelf_haul_ids.first().copied() {
                        if !self.shelf_sel.contains(&id) {
                            self.shelf_sel = self.shelf_haul_ids.clone();
                            self.shelf_anchor = Some(id);
                        }
                    }
                    if !querying {
                        self.shelf_drop = self.haul_drop(pos);
                    }
                    ctx.set_cursor_icon(if on_trash {
                        CursorIcon::Move
                    } else {
                        CursorIcon::Grabbing
                    });
                    ctx.request_repaint();
                } else if touching {
                    if let Some(t0) = self.shelf_hold_t0 {
                        if now - t0 >= WHEEL_HOLD {
                            if let Some(id) = self.shelf_haul_ids.first().copied() {
                                self.open_spine_wheel(id, origin, ctx, false);
                            }
                        }
                    }
                }
            } else if let Some(origin) = self.shelf_band {
                self.shelf_band_now = Some(pos);
                if origin.distance(pos) > 11.0 {
                    self.shelf_band_armed = true;
                    let band = Rect::from_two_pos(origin, pos);
                    let hits: Vec<Uuid> = self
                        .shelf_slots
                        .iter()
                        .filter(|(_, face)| face.intersects(band))
                        .map(|(id, _)| *id)
                        .collect();
                    if self.shelf_band_add {
                        let mut out = self.shelf_band_base.clone();
                        for id in hits {
                            if !out.contains(&id) {
                                out.push(id);
                            }
                        }
                        self.shelf_sel = out;
                    } else {
                        self.shelf_sel = hits;
                    }
                    if let Some(id) = self.shelf_sel.last().copied() {
                        self.shelf_anchor = Some(id);
                    }
                    ctx.request_repaint();
                }
            }
        }
        if released {
            self.shelf_hold_t0 = None;
            if self.shelf_haul_armed {
                let over = !self.shelf_trash
                    && self
                        .shelf_haul_now
                        .is_some_and(|p| self.over_trash_drop_zone(p, screen, 22.0));
                let from_bin = self.haul_from_bin();
                if over && !from_bin {
                    self.begin_toss(ctx.input(|i| i.time));
                    self.shelf_fed = true;
                } else if !querying {
                    if let Some(drop) = self.shelf_drop {
                        let mut ids = self.shelf_haul_ids.clone();
                        ids.sort_by_key(|id| {
                            self.lib
                                .index
                                .notes
                                .iter()
                                .chain(self.lib.index.trash.iter())
                                .find(|n| n.id == *id)
                                .map(|n| n.slot)
                                .unwrap_or(u32::MAX)
                        });
                        match drop {
                            ShelfDrop::Shelf(cell) => {
                                let saved = if from_bin {
                                    self.lib.restore_at(&ids, cell as u32)
                                } else {
                                    self.lib.place_at(&ids, cell as u32)
                                };
                                self.report_save(saved);
                            }
                            ShelfDrop::Bin(cell) => {
                                if from_bin {
                                    let saved = self.lib.place_trash_at(&ids, cell as u32);
                                    self.report_save(saved);
                                } else if self.shelf_trash {
                                    // Into bin cells only while the bin row is open.
                                    let saved = self.lib.trash_at(&ids, cell as u32);
                                    self.report_save(saved);
                                }
                            }
                        }
                        self.shelf_fed = true;
                    }
                }
                self.shelf_haul = None;
                self.shelf_haul_now = None;
                self.shelf_haul_armed = false;
                self.shelf_haul_ids.clear();
                self.shelf_drop = None;
            } else if self.shelf_haul.is_some() {
                self.shelf_haul = None;
                self.shelf_haul_now = None;
                self.shelf_haul_ids.clear();
                self.shelf_drop = None;
            }
            if !self.shelf_band_armed {
                if let Some(origin) = self.shelf_band {
                    let on_slot = self.shelf_slots.iter().any(|(_, r)| r.contains(origin));
                    if !on_slot && !self.shelf_band_add && !chrome && !on_trash {
                        self.shelf_sel.clear();
                        self.shelf_anchor = None;
                    }
                }
                self.shelf_band = None;
                self.shelf_band_now = None;
            }
        }
    }

    fn haul_drop(&self, pos: Pos2) -> Option<ShelfDrop> {
        // Bin cells are drop targets only while the bin row is deliberately open.
        // With the bin closed, shelf → trash goes through the wastebasket logo.
        let bin_ok = self.shelf_trash;
        if bin_ok && self.trash_rect.contains(pos) {
            return None;
        }
        if bin_ok {
            if let Some(i) = self.trash_grid.iter().position(|r| r.contains(pos)) {
                return Some(ShelfDrop::Bin(i));
            }
        }
        if let Some(i) = self.shelf_grid.iter().position(|r| r.contains(pos)) {
            return Some(ShelfDrop::Shelf(i));
        }
        let bin_near = if bin_ok {
            self.trash_grid
                .iter()
                .enumerate()
                .filter(|(_, r)| r.is_positive())
                .min_by(|(_, a), (_, b)| {
                    a.center()
                        .distance_sq(pos)
                        .partial_cmp(&b.center().distance_sq(pos))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        } else {
            None
        };
        let shelf_near = self.shelf_grid.iter().enumerate().min_by(|(_, a), (_, b)| {
            a.center()
                .distance_sq(pos)
                .partial_cmp(&b.center().distance_sq(pos))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        match (bin_near, shelf_near) {
            (Some((i, br)), Some((_, sr))) => {
                if br.center().distance_sq(pos) <= sr.center().distance_sq(pos) {
                    Some(ShelfDrop::Bin(i))
                } else {
                    shelf_near.map(|(i, _)| ShelfDrop::Shelf(i))
                }
            }
            (Some((i, _)), None) => Some(ShelfDrop::Bin(i)),
            (None, Some((i, _))) => Some(ShelfDrop::Shelf(i)),
            (None, None) => None,
        }
    }

    fn begin_toss(&mut self, now: f64) {
        let from = self.shelf_haul_now.unwrap_or(self.trash_mouth);
        let ids = self.shelf_haul_ids.clone();
        for (i, id) in ids.iter().enumerate() {
            let meta = self.lib.index.notes.iter().find(|m| m.id == *id).cloned();
            let Some(meta) = meta else {
                continue;
            };
            self.shelf_toss.push(Toss {
                id: *id,
                from: from + vec2(i as f32 * 10.0, i as f32 * 7.0),
                cover: meta.cover,
                emoji: meta.emoji.clone(),
                t0: now,
                delay: i as f64 * TOSS_STAGGER,
            });
        }
        self.shelf_sel.retain(|id| !ids.contains(id));
    }

    fn shelf_toss_tick(&mut self, ctx: &Context) {
        if self.shelf_toss.is_empty() {
            return;
        }
        let now = ctx.input(|i| i.time);
        let mut done = Vec::new();
        self.shelf_toss.retain(|t| {
            if now - t.t0 - t.delay >= TOSS_DURATION {
                done.push(t.id);
                false
            } else {
                true
            }
        });
        for id in done {
            let saved = self.lib.trash_note(id);
            self.report_save(saved);
        }
        ctx.request_repaint();
    }

    fn wastebasket(&mut self, ctx: &Context) {
        let screen = ctx.screen_rect();
        let active = self.bin_drop_active();
        let reveal = self.trash_reveal;
        let base_center = pos2(screen.right() - 49.0, screen.bottom() - 47.0);
        let corner = pos2(screen.right(), screen.bottom());
        let destination = corner
            - vec2(
                BIN_DROP_RADIUS * QUARTER_DISK_CENTROID,
                BIN_DROP_RADIUS * QUARTER_DISK_CENTROID,
            );
        let center = base_center.lerp(destination, reveal);
        let count = self.lib.index.trash.len() + self.shelf_toss.len();
        let side = egui::lerp(62.0..=104.0, reveal);
        let rect = Rect::from_center_size(center, vec2(side, side));
        if !active {
            self.trash_rect = rect;
        }
        let accepting = self.shelf_haul_armed
            && !self.haul_from_bin()
            && self
                .shelf_haul_now
                .is_some_and(|pos| self.over_trash_drop_zone(pos, screen, 0.0));
        let hover_t = ctx.animate_bool_with_time(
            Id::new("bin-drop-hover"),
            accepting || !self.shelf_toss.is_empty(),
            BIN_HOVER_DURATION,
        );
        let hover = hover_t * hover_t * (3.0 - 2.0 * hover_t);
        let now = ctx.input(|i| i.time);
        let bounce = self
            .shelf_toss
            .iter()
            .map(|t| {
                let u = ((now - t.t0 - t.delay) / TOSS_DURATION).clamp(0.0, 1.0) as f32;
                (u * std::f32::consts::PI).sin()
            })
            .fold(0.0_f32, f32::max);
        let large_scale = (0.75 + 0.33 * hover + 0.04 * bounce) * (0.78 + 0.22 * reveal);
        let mut scale = egui::lerp(0.48..=large_scale, reveal);
        Area::new(Id::new("fab-corbeille"))
            .fixed_pos(rect.min)
            .order(Order::Foreground)
            .interactable(!active)
            .show(ctx, |ui| {
                let (_, resp) = ui.allocate_exact_size(rect.size(), Sense::click());
                if active {
                    ui.disable();
                }
                resp.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Trash")
                });
                if !active {
                    let t = ctx.animate_bool_with_time(
                        Id::new("waste-basket-hover"),
                        resp.hovered() || resp.has_focus(),
                        BIN_HOVER_DURATION,
                    );
                    let t = t * t * (3.0 - 2.0 * t);
                    scale *= 1.0 + 0.05 * t;
                    if resp.has_focus() {
                        ui.painter().rect_stroke(
                            rect.shrink(2.0),
                            CornerRadius::same(12),
                            Stroke::new(1.5, self.look.paper),
                            StrokeKind::Inside,
                        );
                    }
                }
                self.trash_mouth = center - vec2(0.0, 25.0 * scale);
                self.paint_minimalist_bin(
                    ui.painter(),
                    center,
                    scale,
                    if active { hover } else { 0.0 },
                    self.look
                        .paper
                        .gamma_multiply(if active || count > 0 { 0.9 } else { 0.65 }),
                );
                if !active
                    && resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
                    && !self.shelf_fed
                {
                    self.shelf_trash = !self.shelf_trash;
                    self.shelf_sel.clear();
                    self.shelf_anchor = None;
                    if let Scene::Shelf { query } = &mut self.scene {
                        query.clear();
                    }
                }
            });
    }

    fn paper_tex(&mut self, ctx: &Context) -> TextureHandle {
        let key = "canson-paper";
        if let Some(tex) = self.textures.get(key) {
            return tex.clone();
        }
        let bytes = include_bytes!("../assets/canson.jpg");
        let dynimg = image::load_from_memory(bytes).expect("canson");
        let rgba = dynimg.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let img = ColorImage::from_rgba_unmultiplied(size, &rgba);
        let tex = ctx.load_texture(
            key,
            img,
            TextureOptions {
                magnification: TextureFilter::Linear,
                minification: TextureFilter::Linear,
                wrap_mode: TextureWrapMode::Repeat,
                mipmap_mode: Some(TextureFilter::Linear),
            },
        );
        self.textures.insert(key.into(), tex.clone());
        tex
    }

    fn paint_paper_rect(&self, p: &Painter, rect: Rect, tex: &TextureHandle) {
        let uv = Rect::from_min_max(
            Pos2::ZERO,
            pos2(rect.width() / PAPER_TILE, rect.height() / PAPER_TILE),
        );
        p.image(tex.id(), rect, uv, Color32::WHITE);
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_dos_at(
        &mut self,
        ctx: &Context,
        painter: &Painter,
        center: Pos2,
        scale: f32,
        alpha: f32,
        cover: u8,
        emoji: &str,
    ) {
        let w = DOS_W * scale;
        let h = DOS_H * scale;
        let face = Rect::from_center_size(center, vec2(w, h));
        let fade = |c: Color32| {
            Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * alpha) as u8)
        };
        let cloth = fade(self.look.cloth_at(cover));
        let cloth_deep = fade(shade_rgb(self.look.cloth_at(cover), 0.70));
        painter.rect_filled(
            face.translate(vec2(2.0 * scale, 3.0 * scale)),
            CornerRadius::same(8),
            fade(self.look.shadow.gamma_multiply(0.35)),
        );
        painter.rect_filled(face, CornerRadius::same(8), cloth);
        let spine = Rect::from_min_max(face.min, pos2(face.min.x + 13.0 * scale, face.max.y));
        painter.rect_filled(
            spine,
            CornerRadius {
                nw: 8,
                ne: 0,
                sw: 8,
                se: 0,
            },
            cloth_deep,
        );
        painter.rect_stroke(
            face,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, fade(shade_rgb(self.look.cloth_at(cover), 0.48))),
            StrokeKind::Inside,
        );
        let logo = Rect::from_center_size(face.center(), vec2(56.0 * scale, 56.0 * scale));
        if emoji.trim().is_empty() || !self.paint_emoji(ctx, painter, logo, emoji) {
            painter.text(
                face.center(),
                Align2::CENTER_CENTER,
                "+",
                self.look.serif(28.0 * scale),
                fade(self.look.paper.gamma_multiply(0.55)),
            );
        }
    }

    fn shelf_rename_field(&mut self, ctx: &Context) {
        let Some(id) = self.rename_id else {
            self.rename_rect = None;
            return;
        };
        let Some(rect) = self.rename_rect else {
            return;
        };
        let mut commit = false;
        let mut cancel = false;
        Area::new(Id::new("shelf-rename").with(id))
            .fixed_pos(rect.min)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
                ui.visuals_mut().override_text_color = Some(self.look.fg);
                ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.hovered.bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.active.bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.open.bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
                ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
                ui.visuals_mut().widgets.active.bg_stroke = Stroke::NONE;
                ui.visuals_mut().widgets.open.bg_stroke = Stroke::NONE;
                let te = TextEdit::singleline(&mut self.rename_buf)
                    .font(self.look.serif(21.0))
                    .text_color(self.look.fg)
                    .desired_width(rect.width())
                    .frame(false)
                    .margin(Margin::ZERO)
                    .horizontal_align(Align::Center)
                    .vertical_align(Align::Center)
                    .id(Id::new("shelf-rename-edit").with(id));
                let r = ui.add_sized(rect.size(), te);
                let armed = Id::new("shelf-rename-armed").with(id);
                let already = ui
                    .ctx()
                    .data(|d| d.get_temp::<bool>(armed).unwrap_or(false));
                if !already {
                    r.request_focus();
                    ui.ctx().data_mut(|d| d.insert_temp(armed, true));
                }
                if r.lost_focus() {
                    ui.ctx().data_mut(|d| d.remove::<bool>(armed));
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        cancel = true;
                    } else {
                        commit = true;
                    }
                } else if r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    commit = true;
                }
            });
        if commit {
            let title = self.rename_buf.trim().to_string();
            if !title.is_empty() {
                if let Some(mut n) = self.lib.load_note(id) {
                    n.title = title;
                    n.touch();
                    let saved = self.lib.save_note(&n);
                    self.report_save(saved);
                }
            }
            self.rename_id = None;
            self.rename_rect = None;
        } else if cancel {
            self.rename_id = None;
            self.rename_rect = None;
        }
    }

    fn paint_shelf_haul(&mut self, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        let mut ghosts: Vec<(Pos2, f32, f32, u8, String)> = Vec::new();
        if self.shelf_haul_armed {
            if let Some(pos) = self.shelf_haul_now {
                for (i, id) in self.shelf_haul_ids.iter().enumerate() {
                    let meta = self
                        .lib
                        .index
                        .notes
                        .iter()
                        .chain(self.lib.index.trash.iter())
                        .find(|n| n.id == *id);
                    if let Some(m) = meta {
                        ghosts.push((
                            pos + vec2(i as f32 * 11.0, i as f32 * 8.0),
                            0.88,
                            1.0,
                            m.cover,
                            m.emoji.clone(),
                        ));
                    }
                }
            }
        }
        let mouth = self.trash_mouth;
        for t in &self.shelf_toss {
            let u = ((now - t.t0 - t.delay) / TOSS_DURATION).clamp(0.0, 1.0) as f32;
            if now - t.t0 < t.delay {
                ghosts.push((t.from, 0.88, 1.0, t.cover, t.emoji.clone()));
                continue;
            }
            let s = u * u * (3.0 - 2.0 * u);
            let p = t.from.lerp(mouth + vec2(0.0, 26.0), s);
            let lift = (u * std::f32::consts::PI).sin() * 24.0;
            ghosts.push((
                p - vec2(0.0, lift),
                0.88 * (1.0 - s),
                1.0 - s,
                t.cover,
                t.emoji.clone(),
            ));
        }
        if ghosts.is_empty() {
            return;
        }
        Area::new(Id::new("shelf-haul"))
            .fixed_pos(ctx.screen_rect().min)
            .order(Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                let painter = ui.painter().clone();
                for (c, sc, a, cover, em) in ghosts {
                    self.paint_dos_at(ctx, &painter, c, sc, a, cover, &em);
                }
            });
    }

    /// Same well as New, with a question mark in the center.
    fn help_signet(&self, ui: &mut Ui, open: bool) -> Response {
        let size = 56.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let p = ui.painter();
        let paper = mix_col(self.look.paper, self.look.accent, 0.04);
        let well = if resp.hovered() || open {
            mix_col(paper, self.look.accent, 0.08)
        } else {
            paper
        };
        let (face, fg) = self.paint_note_surface(ui, rect.shrink(3.0), &resp, well, 25);
        p.text(
            face.center() + vec2(0.0, 1.0),
            Align2::CENTER_CENTER,
            "?",
            self.look.serif(22.0),
            fg,
        );
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn open_signet(&self, ui: &mut Ui) -> Response {
        let size = 56.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let p = ui.painter();
        let well = if resp.hovered() {
            self.look.desk_edge
        } else {
            self.look.desk_deep
        };
        let (face, fg) = self.paint_note_surface(ui, rect.shrink(3.0), &resp, well, 25);
        paint_open_icon(p, face.center(), fg);
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn quit_signet(&self, ui: &mut Ui) -> Response {
        let size = 56.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let p = ui.painter();
        let well = if resp.hovered() {
            self.look.desk_edge
        } else {
            self.look.desk_deep
        };
        let (face, fg) = self.paint_note_surface(ui, rect.shrink(3.0), &resp, well, 25);
        paint_cross(p, face.center(), 6.5, fg);
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    /// Notebook sheet: the same object for the help card and the marks.
    fn paint_cahier_page(&self, p: &Painter, rect: Rect, paper: &TextureHandle) -> Rect {
        self.paint_cahier_leaf(p, rect, 20.0, 18.0, paper)
    }

    fn paint_cahier_leaf(
        &self,
        p: &Painter,
        rect: Rect,
        top: f32,
        bot: f32,
        paper: &TextureHandle,
    ) -> Rect {
        let ink = self.look.ink;
        p.rect_filled(
            rect.translate(vec2(4.0, 7.0)),
            CornerRadius::same(3),
            self.look.shadow.gamma_multiply(0.42),
        );
        p.rect_filled(
            rect.translate(vec2(1.5, 2.5)),
            CornerRadius::same(3),
            self.look.shadow.gamma_multiply(0.18),
        );
        self.paint_paper_rect(p, rect, paper);
        p.rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0_f32, ink.gamma_multiply(0.12)),
            StrokeKind::Inside,
        );
        Rect::from_min_max(
            pos2(rect.min.x + 22.0, rect.min.y + top),
            pos2(rect.max.x - 22.0, rect.max.y - bot),
        )
    }

    fn fiche_pupitre(&mut self, ui: &mut Ui) {
        let w = 560.0_f32.min(ui.available_width()).max(1.0);
        let stacked = w < 480.0;
        let h = if stacked { 740.0 } else { 420.0 };
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
        let paper = self.paper_tex(ui.ctx());
        let p = ui.painter_at(rect);
        let inner = self.paint_cahier_page(&p, rect, &paper);
        let ink = self.look.ink;
        let mute = ink.gamma_multiply(0.46);
        p.text(
            inner.min,
            Align2::LEFT_TOP,
            "Help",
            self.look.serif(26.0),
            ink,
        );
        let y0 = inner.min.y + 40.0;
        let gap = 22.0;
        let mid = inner.center().x;
        let left = Rect::from_min_max(
            pos2(inner.min.x, y0),
            pos2(
                if stacked {
                    inner.max.x
                } else {
                    mid - gap * 0.5
                },
                inner.max.y,
            ),
        );
        let right = Rect::from_min_max(
            if stacked {
                pos2(inner.min.x, y0 + 380.0)
            } else {
                pos2(mid + gap * 0.5, y0)
            },
            inner.max,
        );
        let keys: &[(&str, &str)] = &[
            ("New", "N"),
            ("Open", "double-click"),
            ("Open file", "Ctrl+O"),
            ("Select", "click"),
            ("Place", "drag"),
            ("Wheel", "right-click"),
            ("Pin", "P"),
            ("Mark", "E"),
            ("Trash", "Del"),
            ("Restore", "R"),
            ("Pan", "space"),
            ("Tear page", "corner"),
            ("Undo", "Ctrl+Z"),
            ("Save", "Ctrl+S"),
            ("Zoom", "+ −"),
        ];
        let hands: &[(&str, &str)] = &[
            ("Draw", "stylus"),
            ("Pan", "finger"),
            ("Scroll", "two fingers"),
            ("Wheel", "hold"),
            ("Undo", "two-finger tap"),
            ("Zoom", "pinch"),
            ("Eraser", "stylus button"),
            ("Lasso", "2nd button"),
            ("Shape", "hold still"),
            ("Remove page", "trash"),
        ];
        self.paint_help_col(&p, left, "keyboard · mouse", keys, ink, mute);
        self.paint_help_col(&p, right, "hands", hands, ink, mute);

        if resp.clicked() {
            self.lib.index.fiche_pliee = true;
            let saved = self.lib.save_index();
            self.report_save(saved);
        }
        resp.on_hover_cursor(CursorIcon::PointingHand);
    }

    fn paint_help_col(
        &self,
        p: &Painter,
        col: Rect,
        head: &str,
        rows: &[(&str, &str)],
        ink: Color32,
        mute: Color32,
    ) {
        p.text(col.min, Align2::LEFT_TOP, head, self.look.mono(10.0), mute);
        let mut y = col.min.y + 22.0;
        for (k, v) in rows {
            p.text(
                pos2(col.min.x, y),
                Align2::LEFT_TOP,
                *k,
                self.look.serif(15.0),
                ink,
            );
            p.text(
                pos2(col.max.x, y + 2.0),
                Align2::RIGHT_TOP,
                *v,
                self.look.mono(11.0),
                mute,
            );
            y += 24.0;
        }
    }

    /// Paints a color icon into `bounds`. False if the bitmap is missing.
    fn paint_emoji(&mut self, ctx: &Context, painter: &Painter, bounds: Rect, emoji: &str) -> bool {
        let Some(ch) = self.emoji.key(emoji) else {
            return false;
        };
        if !self.emoji_tex.contains_key(&ch) {
            let img = if let Some(g) = self.emoji.ensure(ch) {
                ColorImage::from_rgba_unmultiplied([g.width, g.height], &g.rgba)
            } else {
                return false;
            };
            let tex = ctx.load_texture(format!("emoji-{ch}"), img, TextureOptions::LINEAR);
            self.emoji_tex.insert(ch, tex);
        }
        let Some(tex) = self.emoji_tex.get(&ch).cloned() else {
            return false;
        };
        let size = tex.size_vec2();
        let scale = (bounds.width() / size.x).min(bounds.height() / size.y);
        if !scale.is_finite() || scale <= 0.0 {
            return false;
        }
        painter.image(
            tex.id(),
            Rect::from_center_size(bounds.center(), size * scale),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        true
    }

    /// Paper sheet of spine marks: emojis only, scroll to unroll.
    fn ui_emoji_picker(&mut self, ctx: &Context, note_id: Uuid) {
        let mut chosen: Option<String> = None;
        let screen = ctx.screen_rect();
        let sheet_w = (screen.width() * 0.68)
            .clamp(400.0, 700.0)
            .min((screen.width() - 24.0).max(1.0));
        let sheet_h = (screen.height() * 0.80)
            .clamp(440.0, 680.0)
            .min((screen.height() - 24.0).max(1.0));
        let d = self.look.desk;
        // Keep the backdrop and sheet on one modal layer: independently ordered
        // Areas can leave the backdrop above a previously opened sheet.
        let modal = Modal::new(Id::new("emoji-pick"))
            .frame(Frame::NONE)
            .backdrop_color(Color32::from_rgba_unmultiplied(d.r(), d.g(), d.b(), 150))
            .show(ctx, |ui| {
                let paper_col = mix_col(self.look.paper, self.look.accent, 0.028);
                let ink = self.look.ink;
                let (outer, _sheet_resp) =
                    ui.allocate_exact_size(vec2(sheet_w, sheet_h), Sense::hover());
                let paper_tex = self.paper_tex(ui.ctx());
                let p = ui.painter_at(outer);
                p.rect_filled(
                    outer.translate(vec2(4.0, 7.0)),
                    CornerRadius::same(3),
                    self.look.shadow.gamma_multiply(0.42),
                );
                self.paint_paper_rect(&p, outer, &paper_tex);
                p.rect_stroke(
                    outer,
                    CornerRadius::same(3),
                    Stroke::new(1.0_f32, ink.gamma_multiply(0.12)),
                    StrokeKind::Inside,
                );
                let pad = 12.0;
                let inner = outer.shrink(pad);
                ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                    let catalog: Vec<char> = self.emoji.catalog().to_vec();
                    let gap = 4.0;
                    let avail = inner.width().max(40.0);
                    let cols = (avail / (40.0 + gap)).floor().max(1.0) as usize;
                    let cell = (avail - gap * cols.saturating_sub(1) as f32) / cols as f32;
                    let grid_h = inner.height().max(1.0);
                    ui.spacing_mut().item_spacing = vec2(gap, gap);
                    ScrollArea::vertical()
                        .id_salt(("emoji-grid", note_id))
                        .max_height(grid_h)
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                        .scroll_source(ScrollSource {
                            drag: true,
                            scroll_bar: false,
                            mouse_wheel: true,
                        })
                        .show_rows(ui, cell, catalog.len().div_ceil(cols), |ui, rows| {
                            ui.set_width(avail);
                            for row in rows {
                                ui.horizontal(|ui| {
                                    ui.set_width(avail);
                                    ui.spacing_mut().item_spacing = vec2(gap, 0.0);
                                    for ch in
                                        &catalog[row * cols..((row + 1) * cols).min(catalog.len())]
                                    {
                                        let em = ch.to_string();
                                        self.stamp_cell(
                                            ui,
                                            ctx,
                                            paper_col,
                                            ink,
                                            cell,
                                            Some(&em),
                                            &mut chosen,
                                        );
                                    }
                                });
                            }
                        });
                });
            });

        let dismiss = modal.should_close();
        if ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::Paste(_)))) {
            if let Some(s) = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    Event::Paste(t) => Some(t.clone()),
                    _ => None,
                })
            }) {
                let t = s.trim().to_string();
                if self.emoji.key(&t).is_some() {
                    chosen = Some(t);
                } else {
                    self.emoji_query = t;
                }
            }
        }

        if let Some(em) = chosen {
            if let Some(mut n) = self.lib.load_note(note_id) {
                n.emoji = em;
                n.touch();
                let saved = self.lib.save_note(&n);
                self.report_save(saved);
            }
            self.emoji_pick = None;
            self.emoji_query.clear();
        } else if dismiss {
            self.emoji_pick = None;
            self.emoji_query.clear();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stamp_cell(
        &mut self,
        ui: &mut Ui,
        ctx: &Context,
        paper: Color32,
        ink: Color32,
        side: f32,
        em: Option<&str>,
        chosen: &mut Option<String>,
    ) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(3), mix_col(paper, ink, 0.06));
        }
        if let Some(em) = em {
            self.paint_emoji(ctx, p, rect.shrink(5.0), em);
            if resp.clicked() {
                *chosen = Some(em.to_string());
                return true;
            }
        }
        false
    }

    fn haul_from_bin(&self) -> bool {
        self.shelf_haul_ids
            .first()
            .copied()
            .is_some_and(|id| self.lib.is_trashed(id))
    }

    fn bin_open(&self) -> bool {
        self.shelf_trash
    }

    fn bin_drop_active(&self) -> bool {
        !self.shelf_trash && (self.shelf_haul_armed || !self.shelf_toss.is_empty())
    }

    fn over_trash_drop_zone(&self, pos: Pos2, screen: Rect, margin: f32) -> bool {
        if !self.bin_drop_active() {
            return self.trash_rect.expand(margin).contains(pos);
        }
        let dx = screen.right() - pos.x;
        let dy = screen.bottom() - pos.y;
        let radius = BIN_DROP_RADIUS * self.trash_reveal + margin;
        dx >= -margin && dy >= -margin && dx * dx + dy * dy <= radius * radius
    }

    fn shelf_slot() -> Vec2 {
        vec2(
            DOS_W + DOS_PAD * 2.0,
            DOS_PAD + PAPER_PEEK + DOS_H + TITLE_ROW,
        )
    }

    fn close_bin(&mut self, ctx: &Context) {
        self.shelf_trash = false;
        self.shelf_sel.clear();
        self.shelf_anchor = None;
        self.shelf_band = None;
        self.shelf_band_now = None;
        self.shelf_band_armed = false;
        self.shelf_haul = None;
        self.shelf_haul_now = None;
        self.shelf_haul_armed = false;
        self.shelf_haul_ids.clear();
        self.shelf_hold_t0 = None;
        self.shelf_drop = None;
        self.shelf_slots.retain(|(id, _)| !self.lib.is_trashed(*id));
        self.shelf_fed = true;
        ctx.memory_mut(|m| m.surrender_focus(Id::new("close-trash")));
        ctx.request_repaint();
    }

    fn ui_bin_zone(
        &mut self,
        ctx: &Context,
        open: &mut Option<Uuid>,
        select: &mut Option<Uuid>,
        toggle: &mut Option<Uuid>,
        range: &mut Option<Uuid>,
    ) {
        let screen = ctx.screen_rect();
        let compact = screen.width() < 600.0 || screen.height() < 560.0;
        let h = if compact {
            128.0_f32.min((screen.height() - 172.0).max(104.0))
        } else {
            Self::shelf_slot().y + 20.0
        };
        let slot = if compact {
            vec2(216.0, (h - 64.0).clamp(44.0, 64.0))
        } else {
            Self::shelf_slot()
        };
        let target = if self.shelf_trash { h } else { 0.0 };
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        self.bin_panel_height +=
            (target - self.bin_panel_height) * (1.0 - (-BIN_PANEL_SPEED * dt).exp());
        if (target - self.bin_panel_height).abs() < 0.5 {
            self.bin_panel_height = target;
        } else {
            ctx.request_repaint();
        }
        if self.bin_panel_height == 0.0 {
            self.bin_close_rect = Rect::NOTHING;
            return;
        }
        let rust = self.look.rust();
        let fill = mix_col(
            self.look.desk,
            rust,
            if self.look.dark { 0.34 } else { 0.22 },
        );
        let slot_start = self.shelf_slots.len();
        TopBottomPanel::bottom("bin-zone")
            .exact_height(self.bin_panel_height)
            .show_separator_line(false)
            .frame(Frame::NONE.fill(fill))
            .show(ctx, |ui| {
                let shifting = self.shelf_haul_armed;
                let viewport = ui.max_rect();
                // Slide full-size contents below the window edge instead of squeezing them.
                let panel = Rect::from_min_size(viewport.min, vec2(viewport.width(), h));
                let mut content = ui.new_child(UiBuilder::new().max_rect(panel));
                content.set_clip_rect(viewport);
                if !self.shelf_trash {
                    let opacity = content.painter().opacity();
                    content.disable();
                    content.set_opacity(opacity);
                }
                let ui = &mut content;
                let rail = if compact {
                    Rect::from_min_size(
                        panel.min + vec2(8.0, 4.0),
                        vec2(panel.width() - 16.0, 48.0),
                    )
                } else {
                    Rect::from_min_max(
                        pos2((panel.max.x - 180.0).max(panel.min.x), panel.min.y),
                        panel.max,
                    )
                };
                self.trash_rect = rail;
                let grid_rect = if compact {
                    Rect::from_min_max(
                        pos2(panel.left() + 8.0, rail.bottom() + 8.0),
                        panel.max - vec2(8.0, 4.0),
                    )
                } else {
                    Rect::from_min_max(
                        panel.min + vec2(0.0, 8.0),
                        pos2(rail.min.x - 12.0, panel.max.y),
                    )
                };
                let left = if compact { 0.0 } else { 28.0 };
                let pitch = vec2(slot.x + SHELF_GAP, slot.y + SHELF_GAP);
                let cols = TRASH_SLOTS as usize;
                let grid = vec2(left + cols as f32 * pitch.x, slot.y);
                let occ: HashMap<u32, crate::library::NoteMeta> = self
                    .lib
                    .index
                    .trash
                    .iter()
                    .filter(|m| !self.shelf_toss.iter().any(|t| t.id == m.id))
                    .map(|m| (m.slot, m.clone()))
                    .collect();
                ui.scope_builder(UiBuilder::new().max_rect(grid_rect), |ui| {
                    ui.set_clip_rect(grid_rect.intersect(viewport));
                    ScrollArea::new([true, compact])
                        .id_salt("trash-grid")
                        .max_height(grid_rect.height())
                        .scroll_source(ScrollSource {
                            drag: false,
                            scroll_bar: true,
                            mouse_wheel: true,
                        })
                        .show(ui, |ui| {
                            let (full, _) = ui.allocate_exact_size(grid, Sense::hover());
                            let origin = pos2(full.min.x + left, full.min.y);
                            for idx in 0..cols {
                                let cell = Rect::from_min_size(
                                    origin + vec2(idx as f32 * pitch.x, 0.0),
                                    slot,
                                );
                                self.trash_grid.push(cell.intersect(ui.clip_rect()));
                                let meta = occ.get(&(idx as u32)).cloned();
                                let lifted = meta
                                    .as_ref()
                                    .map(|m| shifting && self.shelf_haul_ids.contains(&m.id))
                                    .unwrap_or(false);
                                let hot = shifting && self.shelf_drop == Some(ShelfDrop::Bin(idx));
                                if let Some(meta) = meta.filter(|_| !lifted) {
                                    ui.scope_builder(UiBuilder::new().max_rect(cell), |ui| {
                                        let selected = self.shelf_sel.contains(&meta.id);
                                        let action = if compact {
                                            self.compact_dos(ui, &meta, selected, true, slot)
                                        } else {
                                            self.cahier_dos(ui, &meta, selected, true)
                                        };
                                        match action {
                                            Some(DosAct::Open) => *open = Some(meta.id),
                                            Some(DosAct::Select) => *select = Some(meta.id),
                                            Some(DosAct::Toggle) => *toggle = Some(meta.id),
                                            Some(DosAct::Range) => *range = Some(meta.id),
                                            None => {}
                                        }
                                    });
                                } else if compact {
                                    self.paint_sel_wash(
                                        &ui.painter_at(cell),
                                        cell.shrink(3.0),
                                        hot,
                                    );
                                } else {
                                    self.place_dos(ui, cell, hot);
                                }
                            }
                        });
                });
                self.ui_bin_actions(ctx, ui, rail);
            });
        if !self.shelf_trash {
            self.shelf_slots.truncate(slot_start);
        }
    }

    fn ui_bin_actions(&mut self, ctx: &Context, ui: &mut Ui, rail: Rect) {
        let compact = rail.height() < 90.0;
        let inner = if compact { rail } else { rail.shrink(12.0) };
        let gap = if compact { 8.0 } else { 16.0 };
        let size = if compact {
            vec2((inner.width() - gap) * 0.5, inner.height())
        } else {
            vec2(inner.width(), (inner.height() - gap) * 0.5)
        };
        let empty = Rect::from_min_size(inner.min, size);
        let close = Rect::from_min_size(
            if compact {
                pos2(empty.right() + gap, inner.top())
            } else {
                pos2(inner.min.x, empty.max.y + gap)
            },
            size,
        );
        self.bin_close_rect = close;
        ui.scope_builder(UiBuilder::new().max_rect(close), |ui| {
            if self.shelf_haul.is_some() || self.shelf_haul_armed {
                ui.disable();
            }
            let resp = ui.interact(close, Id::new("close-trash"), Sense::click());
            resp.widget_info(|| {
                WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Close trash")
            });
            let (c, color, _) = self.paint_bin_action(ui, close, &resp, "Close trash");
            ui.painter().add(Shape::line(
                vec![
                    c + vec2(-16.0, -5.0),
                    c + vec2(0.0, 7.0),
                    c + vec2(16.0, -5.0),
                ],
                Stroke::new(2.5, color),
            ));
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                self.close_bin(ctx);
            }
        });
        self.ui_empty_bin(ui, empty);
    }

    fn paint_bin_action(
        &self,
        ui: &Ui,
        hit: Rect,
        resp: &Response,
        label: &str,
    ) -> (Pos2, Color32, f32) {
        let hover = ui.ctx().animate_bool_with_time(
            resp.id.with("hover"),
            ui.is_enabled() && (resp.hovered() || resp.has_focus()),
            BIN_HOVER_DURATION,
        );
        let e = hover * hover * (3.0 - 2.0 * hover);
        let p = ui.painter();
        let fill = mix_col(self.look.paper, self.look.rust(), 0.56 - 0.08 * e);
        let (face, color) = self.paint_note_surface(ui, hit.shrink(2.0), resp, fill, 12);
        let compact = hit.height() < 90.0;
        let c = if compact {
            pos2(hit.left() + 23.0, face.center().y)
        } else {
            face.center() + vec2(0.0, -14.0)
        };
        if compact {
            p.text(
                pos2(hit.left() + 46.0, c.y),
                Align2::LEFT_CENTER,
                label,
                self.look.mono(10.5),
                color,
            );
        } else {
            p.text(
                c + vec2(0.0, 40.0),
                Align2::CENTER_CENTER,
                label,
                self.look.mono(12.0),
                color,
            );
        }
        (c, color, e)
    }

    fn ui_empty_bin(&mut self, ui: &mut Ui, hit: Rect) {
        let empty = self.lib.index.trash.is_empty();
        ui.scope_builder(UiBuilder::new().max_rect(hit), |ui| {
            if empty || self.shelf_haul_armed {
                ui.disable();
            }
            let resp = ui.interact(hit, Id::new("empty-trash"), Sense::click());
            resp.widget_info(|| {
                WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Empty trash")
            });
            let (center, color, e) = self.paint_bin_action(
                ui,
                hit,
                &resp,
                if empty {
                    "Trash is empty"
                } else {
                    "Empty trash"
                },
            );
            let press = if resp.is_pointer_button_down_on() {
                0.95
            } else {
                1.0
            };
            self.paint_minimalist_bin(
                ui.painter(),
                center,
                (if hit.height() < 90.0 { 0.24 } else { 0.62 } + 0.025 * e) * press,
                e,
                color,
            );
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                let saved = self.lib.empty_trash();
                self.report_save(saved);
                self.shelf_sel.clear();
                self.shelf_anchor = None;
                self.shelf_fed = true;
            }
        });
    }

    fn paint_trash_drop_zone(&mut self, ctx: &Context) {
        let screen = ctx.screen_rect();
        let active = self.bin_drop_active();
        let opening =
            ctx.animate_bool_with_time(Id::new("bin-drop-zone-open"), active, BIN_REVEAL_DURATION);
        let e = opening * opening * (3.0 - 2.0 * opening);
        self.trash_reveal = e;
        if self.bin_open() || e <= 0.0 {
            return;
        }
        let radius = BIN_DROP_RADIUS * e;
        let corner = pos2(screen.right(), screen.bottom());
        self.trash_rect = Rect::from_min_max(corner - vec2(radius, radius), corner);
        let rust = self.look.rust();
        let fill = mix_col(
            self.look.desk,
            rust,
            if self.look.dark { 0.34 } else { 0.22 },
        )
        .gamma_multiply(e);
        // Keep the sector below the dragged notebook and its toss animation.
        let painter = ctx.layer_painter(LayerId::new(Order::Middle, Id::new("bin-drop-zone")));
        let mut arc = Vec::with_capacity(25);
        for i in 0..=24 {
            let angle = std::f32::consts::FRAC_PI_2 * (1.0 - i as f32 / 24.0);
            arc.push(corner - vec2(radius * angle.cos(), radius * angle.sin()));
        }
        let mut wedge = vec![corner];
        wedge.extend(arc.iter().copied());
        painter.add(Shape::convex_polygon(wedge, fill, Stroke::NONE));
        if e > 0.02 {
            painter.add(Shape::line(
                arc,
                Stroke::new(1.2, shade_rgb(rust, 1.18).gamma_multiply(0.48 * e)),
            ));
        }
    }

    fn paint_minimalist_bin(&self, p: &Painter, center: Pos2, scale: f32, e: f32, color: Color32) {
        let body = |x: f32, y: f32| center + vec2(x, y) * scale;
        let angle = -0.24 * e;
        let lid = |x: f32, y: f32| {
            let hinge = vec2(-45.0, -25.0);
            let d = vec2(x, y) - hinge;
            let rotated = vec2(
                d.x * angle.cos() - d.y * angle.sin(),
                d.x * angle.sin() + d.y * angle.cos(),
            );
            center + (hinge + rotated - vec2(0.0, 6.0 * e)) * scale
        };
        let stroke = Stroke::new(5.0 * scale, color);

        // The lid lifts toward the incoming notebook.
        p.add(Shape::line(
            vec![
                lid(-14.0, -31.0),
                lid(-14.0, -39.0),
                lid(14.0, -39.0),
                lid(14.0, -31.0),
            ],
            stroke,
        ));
        p.add(Shape::line(
            vec![lid(-45.0, -25.0), lid(45.0, -25.0)],
            stroke,
        ));

        // Slightly tapered bin body with three simple ribs.
        p.add(Shape::line(
            vec![
                body(-36.0, -21.0),
                body(-28.0, 39.0),
                body(28.0, 39.0),
                body(36.0, -21.0),
            ],
            stroke,
        ));
        for x in [-14.0, 0.0, 14.0] {
            let rib = Stroke::new(4.0 * scale, color);
            let a = body(x, -12.0);
            let b = body(x, 28.0);
            p.add(Shape::line(vec![a, b], rib));
        }
    }

    fn paint_sel_wash(&self, p: &Painter, rect: Rect, strong: bool) {
        let (r, g, b) = if self.look.dark {
            (255_u8, 255, 255)
        } else {
            (self.look.ink.r(), self.look.ink.g(), self.look.ink.b())
        };
        let fill = if strong {
            if self.look.dark {
                28
            } else {
                22
            }
        } else if self.look.dark {
            18
        } else {
            16
        };
        let line = if strong {
            if self.look.dark {
                78
            } else {
                56
            }
        } else if self.look.dark {
            52
        } else {
            36
        };
        p.rect_filled(
            rect,
            CornerRadius::same(11),
            Color32::from_rgba_unmultiplied(r, g, b, fill),
        );
        p.rect_stroke(
            rect,
            CornerRadius::same(11),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(r, g, b, line)),
            StrokeKind::Inside,
        );
    }

    fn place_dos(&self, ui: &mut Ui, cell: Rect, hot: bool) {
        let face = Rect::from_min_size(
            pos2(
                cell.center().x - DOS_W * 0.5,
                cell.min.y + DOS_PAD + PAPER_PEEK,
            ),
            vec2(DOS_W, DOS_H),
        );
        self.paint_sel_wash(&ui.painter_at(cell), face.expand(5.0), hot);
    }

    fn compact_dos(
        &mut self,
        ui: &mut Ui,
        meta: &crate::library::NoteMeta,
        selected: bool,
        in_bin: bool,
        size: Vec2,
    ) -> Option<DosAct> {
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        self.shelf_slots
            .push((meta.id, rect.intersect(ui.clip_rect())));
        let p = ui.painter_at(rect);
        if selected {
            self.paint_sel_wash(&p, rect.shrink(2.0), false);
        }
        let scale = ((size.y - 12.0) / DOS_H).min(0.30);
        self.paint_dos_at(
            ui.ctx(),
            &p,
            pos2(rect.left() + 27.0, rect.center().y),
            scale,
            1.0,
            meta.cover,
            &meta.emoji,
        );
        let title = Rect::from_min_max(
            pos2(rect.left() + 56.0, rect.top() + 8.0),
            rect.max - vec2(6.0, 8.0),
        );
        let mut job = egui::text::LayoutJob::simple_singleline(
            meta.title.clone(),
            self.look.serif(16.0),
            self.look.fg,
        );
        job.wrap.max_width = title.width();
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = p.layout_job(job);
        p.galley(
            pos2(title.left(), title.center().y - galley.size().y * 0.5),
            galley,
            self.look.fg,
        );
        if meta.pinned {
            p.circle_filled(rect.min + vec2(44.0, 10.0), 3.0, self.look.accent);
        }
        let on_title = response.hover_pos().is_some_and(|pos| title.contains(pos));
        if !in_bin && on_title && response.clicked() {
            self.rename_id = Some(meta.id);
            self.rename_buf = meta.title.clone();
            self.rename_rect = Some(title);
            return None;
        }
        if self.rename_id == Some(meta.id) {
            self.rename_rect = Some(title);
        }
        if response.double_clicked() && !in_bin && !on_title {
            return Some(DosAct::Open);
        }
        response.clone().on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() && !self.shelf_haul_armed {
            let modifiers = ui.input(|i| i.modifiers);
            Some(if modifiers.command {
                DosAct::Toggle
            } else if modifiers.shift {
                DosAct::Range
            } else {
                DosAct::Select
            })
        } else {
            None
        }
    }

    fn cahier_dos(
        &mut self,
        ui: &mut Ui,
        meta: &crate::library::NoteMeta,
        selected: bool,
        in_bin: bool,
    ) -> Option<DosAct> {
        let slot = vec2(
            DOS_W + DOS_PAD * 2.0,
            DOS_PAD + PAPER_PEEK + DOS_H + TITLE_ROW,
        );
        let (slot_rect, resp) = ui.allocate_exact_size(slot, Sense::click());
        let id = Id::new("cahier-dos").with(meta.id);
        let renaming = self.rename_id == Some(meta.id) && !in_bin;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over = !renaming && pointer.is_some_and(|p| slot_rect.contains(p));
        if over {
            ui.ctx().request_repaint();
        }
        let lift_t = ui.ctx().animate_bool_with_time(id.with("peek"), over, 0.16);
        let lift = lift_t * lift_t * (3.0 - 2.0 * lift_t);
        let e = if in_bin { lift * 0.2 } else { lift };
        let cloth = if in_bin {
            mix_col(self.look.cloth_at(meta.cover), self.look.rust(), 0.16)
        } else {
            self.look.cloth_at(meta.cover)
        };
        let cloth_deep = shade_rgb(cloth, 0.70);
        let cloth_edge = shade_rgb(cloth, 0.48);
        let paper = self.look.paper;

        let face = Rect::from_min_size(
            pos2(
                slot_rect.center().x - DOS_W * 0.5,
                slot_rect.min.y + DOS_PAD + PAPER_PEEK,
            ),
            vec2(DOS_W, DOS_H),
        );
        let painter = ui.painter_at(slot_rect);
        let hit = Rect::from_min_max(face.min, pos2(face.max.x, face.max.y + TITLE_ROW));
        self.shelf_slots
            .push((meta.id, hit.intersect(ui.clip_rect())));
        let lifted = self.shelf_haul_armed && self.shelf_haul_ids.contains(&meta.id);
        if lifted {
            painter.rect_filled(
                face.translate(vec2(2.0, 4.0)),
                CornerRadius::same(8),
                self.look.shadow.gamma_multiply(0.18),
            );
            return None;
        }

        if selected {
            self.paint_sel_wash(&painter, face.expand(5.0), false);
        }

        painter.rect_filled(
            face.translate(vec2(3.0, 5.0 + 1.5 * e)),
            CornerRadius::same(8),
            self.look.shadow.gamma_multiply(0.40 + 0.18 * e),
        );

        let peek = e * (PAPER_PEEK - 3.0);
        if peek > 1.0 {
            let sheet = Rect::from_min_max(
                pos2(face.min.x + 18.0, face.min.y - peek),
                pos2(face.max.x - 14.0, face.min.y + 8.0),
            );
            painter.rect_filled(
                sheet,
                CornerRadius {
                    nw: 2,
                    ne: 2,
                    sw: 0,
                    se: 0,
                },
                paper,
            );
            if e > 0.25 {
                let mut y = sheet.min.y + 8.0;
                while y < face.min.y - 2.0 {
                    painter.line_segment(
                        [pos2(sheet.min.x + 6.0, y), pos2(sheet.max.x - 6.0, y)],
                        Stroke::new(1.0_f32, self.look.paper_rule),
                    );
                    y += 9.0;
                }
            }
        }

        painter.rect_filled(face, CornerRadius::same(8), cloth);
        let spine = Rect::from_min_max(face.min, pos2(face.min.x + 13.0, face.max.y));
        painter.rect_filled(
            spine,
            CornerRadius {
                nw: 8,
                ne: 0,
                sw: 8,
                se: 0,
            },
            cloth_deep,
        );
        painter.line_segment(
            [
                pos2(spine.max.x, face.min.y + 10.0),
                pos2(spine.max.x, face.max.y - 10.0),
            ],
            Stroke::new(1.1_f32, cloth_edge),
        );
        painter.rect_stroke(
            face,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, cloth_edge),
            StrokeKind::Inside,
        );
        for i in 0..3 {
            let o = i as f32 * 1.5;
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(face.max.x - 1.0 + o, face.min.y + 10.0),
                    pos2(face.max.x + 2.2 + o, face.max.y - 10.0),
                ),
                CornerRadius::same(1),
                shade_rgb(paper, 0.94 - i as f32 * 0.04),
            );
        }

        // Center of the board (the title sits under the spine).
        let cover = Rect::from_min_max(
            pos2(face.min.x + 13.0, face.min.y + 12.0),
            pos2(face.max.x - 6.0, face.max.y - 12.0),
        );
        let mark = cover.center();
        let logo_r = Rect::from_center_size(mark, vec2(56.0, 56.0));
        let emoji = meta.emoji.trim();
        let painted = !emoji.is_empty() && self.paint_emoji(ui.ctx(), &painter, logo_r, emoji);
        if !painted {
            painter.text(
                mark,
                Align2::CENTER_CENTER,
                "+",
                self.look.serif(28.0),
                paper.gamma_multiply(0.55),
            );
        }

        if meta.pinned {
            let x = face.max.x - 16.0;
            let ribbon = [
                pos2(x - 3.4, face.min.y),
                pos2(x + 3.4, face.min.y),
                pos2(x + 3.4, face.min.y + 26.0),
                pos2(x, face.min.y + 31.0),
                pos2(x - 3.4, face.min.y + 26.0),
            ];
            painter.add(Shape::convex_polygon(
                ribbon.to_vec(),
                self.look.accent,
                Stroke::NONE,
            ));
        }

        let title_rect = Rect::from_min_size(
            pos2(face.min.x, face.max.y + 10.0),
            vec2(face.width(), TITLE_ROW - 10.0),
        );
        if renaming {
            self.rename_rect = Some(title_rect);
        } else {
            let title: String = {
                let t = meta.title.as_str();
                if t.chars().count() > 14 {
                    format!("{}…", t.chars().take(12).collect::<String>())
                } else {
                    t.to_string()
                }
            };
            if selected {
                let (r, g, b) = if self.look.dark {
                    (255_u8, 255, 255)
                } else {
                    (self.look.ink.r(), self.look.ink.g(), self.look.ink.b())
                };
                let name = Rect::from_center_size(
                    title_rect.center(),
                    vec2((title_rect.width() - 4.0).max(56.0), 30.0),
                );
                painter.rect_filled(
                    name,
                    CornerRadius::same(7),
                    Color32::from_rgba_unmultiplied(r, g, b, if self.look.dark { 20 } else { 16 }),
                );
            }
            painter.text(
                title_rect.center(),
                Align2::CENTER_CENTER,
                title,
                self.look.serif(21.0),
                self.look.fg,
            );
            if !in_bin {
                let title_resp = ui.interact(title_rect, id.with("title"), Sense::click());
                if title_resp.clicked() {
                    self.rename_buf = meta.title.clone();
                    self.rename_id = Some(meta.id);
                    self.rename_rect = Some(title_rect);
                }
                title_resp.on_hover_cursor(CursorIcon::Text);
            }
        }

        let mut act = None;
        if act.is_none() && !renaming && !resp.secondary_clicked() && self.spine_wheel.is_none() {
            let on_title = pointer.is_some_and(|p| title_rect.contains(p));
            let mods = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            if resp.double_clicked() && !on_title && !in_bin {
                act = Some(DosAct::Open);
            } else if resp.clicked()
                && !on_title
                && !self.shelf_band_armed
                && !self.shelf_haul_armed
                && !self.shelf_fed
            {
                act = Some(if mods.0 {
                    DosAct::Toggle
                } else if mods.1 {
                    DosAct::Range
                } else {
                    DosAct::Select
                });
            }
        }
        if !renaming {
            resp.clone().on_hover_cursor(CursorIcon::PointingHand);
        }
        act
    }

    fn open_spine_wheel(&mut self, id: Uuid, pos: Pos2, ctx: &Context, secondary: bool) {
        let now = ctx.input(|i| i.time);
        let touch_origin = if secondary { None } else { Some(pos) };
        let (pos, _) = fitted_wheel(ctx.screen_rect(), pos);
        self.spine_wheel = Some(SpineWheel {
            id,
            origin: pos,
            t0: now,
            colors: false,
            hover: None,
            secondary,
            color_hold_t0: None,
            touch_origin,
        });
        self.shelf_haul = None;
        self.shelf_haul_now = None;
        self.shelf_haul_armed = false;
        self.shelf_haul_ids.clear();
        self.shelf_drop = None;
        self.shelf_hold_t0 = None;
        self.shelf_fed = true;
    }

    fn spine_wheel_tick(&mut self, ctx: &Context) {
        let Some(wheel) = self.spine_wheel.as_mut() else {
            return;
        };
        let (origin, scale) = fitted_wheel(ctx.screen_rect(), wheel.origin);
        wheel.origin = origin;
        let n_cloth = self.look.cloth.len().max(1);
        let secondary = wheel.secondary;
        let (pos, down, released, primary_pressed, now) = ctx.input(|i| {
            (
                i.pointer.interact_pos().or(i.pointer.hover_pos()),
                if secondary {
                    i.pointer.button_down(PointerButton::Secondary)
                } else {
                    i.pointer.primary_down()
                },
                if secondary {
                    i.pointer.button_released(PointerButton::Secondary)
                } else {
                    i.pointer.primary_released()
                },
                i.pointer.primary_pressed(),
                i.time,
            )
        });
        let pos = pos
            .filter(|p| {
                if let Some(start) = wheel.touch_origin {
                    if p.distance(start) < 8.0 {
                        return false;
                    }
                    wheel.touch_origin = None;
                }
                true
            })
            .map(|p| origin + (p - origin) / scale);
        if let Some(pos) = pos {
            wheel.hover = wheel_hit(wheel.origin, pos, wheel.colors, n_cloth);
            // Finger: linger on Color to open the cloth ring.
            // Mouse: left-click Color instead (handled below).
            if !secondary && !wheel.colors {
                if wheel.hover == Some(WheelPick::Color) {
                    let t0 = *wheel.color_hold_t0.get_or_insert(now);
                    if now - t0 >= COLOR_HOLD {
                        wheel.colors = true;
                        wheel.color_hold_t0 = None;
                        wheel.hover = wheel_hit(wheel.origin, pos, true, n_cloth);
                    }
                } else {
                    wheel.color_hold_t0 = None;
                }
            }
        }

        if secondary {
            // Mouse: right-click opened the wheel; left-click activates.
            if primary_pressed {
                let id = wheel.id;
                let pick = wheel.hover;
                let origin = wheel.origin;
                let colors = wheel.colors;
                match pick {
                    Some(WheelPick::Color) if !colors => {
                        wheel.colors = true;
                        wheel.color_hold_t0 = None;
                        if let Some(pos) = pos {
                            wheel.hover = wheel_hit(wheel.origin, pos, true, n_cloth);
                        }
                        self.shelf_fed = true;
                    }
                    Some(_) => {
                        self.spine_wheel = None;
                        self.shelf_fed = true;
                        self.apply_spine_wheel(id, pick, origin, now);
                    }
                    None => {
                        // On the cloth ring, center click steps back to actions.
                        let at_center = pos.is_some_and(|p| (p - origin).length() < WHEEL_IN);
                        if colors && at_center {
                            wheel.colors = false;
                            wheel.color_hold_t0 = None;
                            if let Some(pos) = pos {
                                wheel.hover = wheel_hit(origin, pos, false, n_cloth);
                            }
                            self.shelf_fed = true;
                        } else {
                            self.spine_wheel = None;
                            self.shelf_fed = true;
                        }
                    }
                }
            }
            ctx.request_repaint();
            return;
        }

        // Finger: commit on release.
        if released || !down {
            let id = wheel.id;
            let pick = wheel.hover;
            let origin = wheel.origin;
            self.spine_wheel = None;
            self.shelf_fed = true;
            self.apply_spine_wheel(id, pick, origin, now);
        }
        ctx.request_repaint();
    }

    fn apply_spine_wheel(&mut self, id: Uuid, pick: Option<WheelPick>, origin: Pos2, t: f64) {
        match pick {
            Some(WheelPick::Pin) => {
                if let Some(mut note) = self.lib.load_note(id) {
                    note.pinned = !note.pinned;
                    let pinned = note.pinned;
                    let saved = self.lib.save_note(&note);
                    self.report_save(saved);
                    if pinned {
                        let saved = self.lib.bring_front(id);
                        self.report_save(saved);
                    }
                }
            }
            Some(WheelPick::Mark) => {
                self.emoji_pick = Some(id);
                self.rename_id = None;
            }
            Some(WheelPick::Trash) => {
                self.shelf_haul_ids = vec![id];
                self.shelf_haul_now = Some(origin);
                self.begin_toss(t);
                self.shelf_fed = true;
            }
            Some(WheelPick::Cloth(i)) => {
                if let Some(mut note) = self.lib.load_note(id) {
                    note.cover = i;
                    note.touch();
                    let saved = self.lib.save_note(&note);
                    self.report_save(saved);
                }
            }
            Some(WheelPick::Color) | None => {}
        }
    }

    fn paint_spine_wheel(&mut self, ctx: &Context) {
        let Some(wheel) = self.spine_wheel.as_ref() else {
            return;
        };
        let origin = wheel.origin;
        let colors = wheel.colors;
        let hover = wheel.hover;
        let t0 = wheel.t0;
        let now = ctx.input(|i| i.time);
        let appear = ((now - t0) / 0.05).clamp(0.0, 1.0) as f32;
        let s = 1.0 - (1.0 - appear) * (1.0 - appear);
        let (_, scale) = fitted_wheel(ctx.screen_rect(), origin);
        let r0 = WHEEL_IN * s * scale;
        let r1 = WHEEL_OUT * s * scale;
        if r0 < 1.0 {
            return;
        }
        let n_cloth = self.look.cloth.len().max(1);
        let paper = self.paper_tex(ctx);

        Area::new(Id::new("spine-wheel"))
            .fixed_pos(ctx.screen_rect().min)
            .order(Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                let p = ui.painter().clone();
                p.circle_filled(
                    origin,
                    r1 + 14.0,
                    Color32::from_black_alpha(if self.look.dark { 40 } else { 22 }),
                );
                if colors {
                    for i in 0..n_cloth {
                        let cloth = self.look.cloth_at(i as u8);
                        let hot = hover == Some(WheelPick::Cloth(i as u8));
                        let fill = if hot { shade_rgb(cloth, 1.22) } else { cloth };
                        p.add(wedge_mesh(origin, r0, r1, n_cloth, i, fill, None));
                        if hot {
                            p.add(wedge_stroke(origin, r0, r1, n_cloth, i, self.look.ink));
                        }
                    }
                } else {
                    let picks = [
                        WheelPick::Pin,
                        WheelPick::Color,
                        WheelPick::Trash,
                        WheelPick::Mark,
                    ];
                    for (i, pick) in picks.into_iter().enumerate() {
                        let hot = hover == Some(pick);
                        let tint = if hot {
                            mix_col(Color32::WHITE, self.look.accent, 0.12)
                        } else {
                            Color32::WHITE
                        };
                        p.add(wedge_mesh(
                            origin,
                            r0,
                            r1,
                            4,
                            i,
                            tint,
                            Some((paper.id(), PAPER_TILE)),
                        ));
                        if hot {
                            p.add(wedge_stroke(origin, r0, r1, 4, i, self.look.ink));
                        }
                    }
                }
                p.circle_filled(origin, r0 - 1.0, self.look.desk);
                p.circle_stroke(
                    origin,
                    r0,
                    Stroke::new(1.4_f32, self.look.ink.gamma_multiply(0.28)),
                );
                p.circle_stroke(
                    origin,
                    r1,
                    Stroke::new(1.4_f32, self.look.ink.gamma_multiply(0.22)),
                );

                if colors {
                    for i in 0..n_cloth {
                        let c = slice_mid(origin, i, n_cloth, (r0 + r1) * 0.52);
                        p.circle_filled(c, 14.0, shade_rgb(self.look.cloth_at(i as u8), 1.08));
                        p.circle_stroke(
                            c,
                            14.0,
                            Stroke::new(1.3_f32, self.look.ink.gamma_multiply(0.45)),
                        );
                    }
                } else {
                    let picks = [
                        WheelPick::Pin,
                        WheelPick::Color,
                        WheelPick::Trash,
                        WheelPick::Mark,
                    ];
                    let fills = [
                        self.tool_well_fill(Tool::Highlighter, false),
                        self.look.accent,
                        self.look.rust(),
                        self.look.green(),
                    ];
                    for (i, pick) in picks.into_iter().enumerate() {
                        let c = slice_mid(origin, i, 4, r0 + (r1 - r0) * 0.55);
                        let hot = hover == Some(pick);
                        let fill = if hot {
                            mix_col(fills[i], self.look.paper, 0.14)
                        } else {
                            fills[i]
                        };
                        let bounds = Rect::from_center_size(c, vec2(44.0, 44.0));
                        let (face, fg) = self.paint_raised_face(&p, bounds, fill, 22, hot);
                        match pick {
                            WheelPick::Pin => paint_pin_icon(&p, face.center(), fg),
                            WheelPick::Color => self.paint_palette_icon(&p, face.center(), fg),
                            WheelPick::Trash => {
                                self.paint_minimalist_bin(&p, face.center(), 0.27, 0.0, fg)
                            }
                            WheelPick::Mark => paint_smile_icon(&p, face.center(), fg),
                            WheelPick::Cloth(_) => unreachable!(),
                        }
                    }
                }
            });
    }

    fn menu_row(
        &self,
        ui: &mut Ui,
        label: &str,
        hint: &str,
        checked: bool,
        destructive: bool,
    ) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
        resp.widget_info(|| {
            WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), checked, label)
        });
        if resp.gained_focus() {
            resp.scroll_to_me(Some(Align::Center));
        }
        let fill = if destructive {
            mix_col(self.look.paper, self.look.rust(), 0.55)
        } else if checked {
            mix_col(self.look.paper, self.look.green(), 0.55)
        } else {
            self.look.paper
        };
        let (face, fg) = self.paint_note_surface(ui, rect.shrink(2.0), &resp, fill, 12);
        if checked {
            paint_check(ui.painter(), pos2(face.min.x + 16.0, face.center().y), fg);
        } else if destructive {
            self.paint_minimalist_bin(
                ui.painter(),
                pos2(face.min.x + 16.0, face.center().y),
                0.17,
                0.0,
                fg,
            );
        }
        ui.painter().text(
            pos2(face.min.x + 32.0, face.center().y),
            Align2::LEFT_CENTER,
            label,
            self.look.mono(13.0),
            fg,
        );
        if !hint.is_empty() {
            ui.painter().text(
                pos2(face.max.x - 12.0, face.center().y),
                Align2::RIGHT_CENTER,
                hint,
                self.look.mono(11.0),
                fg.gamma_multiply(0.62),
            );
        }
        resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
    }

    fn menu_sep(&self, ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
        ui.painter().hline(
            rect.x_range().shrink(10.0),
            rect.center().y,
            Stroke::new(1.0_f32, self.look.paper.gamma_multiply(0.16)),
        );
    }

    fn round_well(
        &self,
        ui: &mut Ui,
        size: f32,
        fill: Color32,
        paint: impl FnOnce(&Painter, Pos2, Color32),
    ) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let (face, fg) =
            self.paint_note_surface(ui, rect.shrink(2.0), &resp, fill, (size * 0.5) as u8);
        paint(ui.painter(), face.center(), fg);
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    /// Pigmented stationery, using the same live theme as the shelf and ink tray.
    /// These are control colours, independent of the user's selected drawing ink.
    fn tool_well_fill(&self, tool: Tool, active: bool) -> Color32 {
        let pigment = match tool {
            Tool::Fineliner => 7,   // blue
            Tool::Brush => 8,       // violet
            Tool::Pencil => 3,      // terracotta
            Tool::Highlighter => 4, // yellow
            Tool::EraserStroke | Tool::EraserArea => 2,
            Tool::Lasso => 6, // teal
            Tool::Text => 9,  // warm brown
            Tool::Image => 5, // green
        };
        let color = self
            .look
            .inks
            .get(pigment)
            .copied()
            .unwrap_or(self.look.accent);
        mix_col(self.look.paper, color, if active { 1.0 } else { 0.56 })
    }

    fn note_case_color(&self) -> Color32 {
        let cover = self.note.as_ref().map_or(0, |note| note.cover);
        mix_col(self.look.desk_deep, self.look.cloth_at(cover), 0.68)
    }

    fn note_popup_frame(&self) -> Frame {
        Frame::NONE
            .fill(self.note_case_color())
            .stroke(Stroke::new(
                1.0,
                mix_col(self.note_case_color(), self.look.paper, 0.22),
            ))
            .corner_radius(22)
            .shadow(egui::epaint::Shadow {
                offset: [0, 4],
                blur: 8,
                spread: 0,
                color: self.look.shadow.gamma_multiply(0.65),
            })
            .inner_margin(Margin::same(8))
    }

    fn paint_note_surface(
        &self,
        ui: &Ui,
        rect: Rect,
        resp: &Response,
        fill: Color32,
        radius: u8,
    ) -> (Rect, Color32) {
        if resp.gained_focus() {
            resp.scroll_to_me(Some(Align::Center));
        }
        let p = ui.painter();
        let fill = if resp.hovered() {
            mix_col(fill, self.look.paper, 0.14)
        } else {
            fill
        };
        let (face, fg) =
            self.paint_raised_face(p, rect, fill, radius, resp.is_pointer_button_down_on());
        if resp.has_focus() {
            p.rect_stroke(
                rect.expand(1.5),
                CornerRadius::same(radius.saturating_add(2)),
                Stroke::new(1.5, self.look.accent),
                StrokeKind::Outside,
            );
        }
        (face, fg)
    }

    fn paint_raised_face(
        &self,
        p: &Painter,
        rect: Rect,
        fill: Color32,
        radius: u8,
        down: bool,
    ) -> (Rect, Color32) {
        let face = rect.translate(vec2(0.0, if down { 1.5 } else { -0.5 }));
        let fg = well_glyph(fill, self.look.paper, self.look.ink);
        p.rect_filled(
            rect.translate(vec2(0.0, 2.5)),
            CornerRadius::same(radius),
            self.look.shadow.gamma_multiply(0.65),
        );
        p.rect_filled(face, CornerRadius::same(radius), fill);
        p.rect_stroke(
            face,
            CornerRadius::same(radius),
            Stroke::new(1.0, self.look.ink.gamma_multiply(0.24)),
            StrokeKind::Inside,
        );
        p.rect_stroke(
            face.shrink(3.5),
            CornerRadius::same(radius.saturating_sub(3)),
            Stroke::new(1.0, fg.gamma_multiply(0.14)),
            StrokeKind::Inside,
        );
        (face, fg)
    }

    fn note_action(&self, ui: &mut Ui, label: &str, width: f32, destructive: bool) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
        let fill = if destructive {
            self.look.rust()
        } else {
            self.look.paper
        };
        let (face, fg) = self.paint_note_surface(ui, rect.shrink(2.0), &resp, fill, 12);
        ui.painter().text(
            face.center(),
            Align2::CENTER_CENTER,
            label,
            self.look.mono(13.0),
            fg,
        );
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    /// A simple return arrow on the same paper well as the other note controls.
    fn shelf_exit_btn(&self, ui: &mut Ui) -> Response {
        let resp = self.round_well(ui, self.chrome_btn(), self.look.paper, paint_back);
        resp.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Back to home")
        });
        resp
    }

    fn inkwell(&self, ui: &mut Ui) -> Response {
        let size = 56.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let p = ui.painter();
        let g = self.look.green();
        let well = if resp.hovered() {
            shade_rgb(g, 1.14)
        } else {
            g
        };
        let (face, fg) = self.paint_note_surface(ui, rect.shrink(3.0), &resp, well, 25);
        paint_plus(p, face.center(), fg);
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn ui_desk(&mut self, ctx: &Context) {
        let compact = ctx.screen_rect().width() < 760.0;
        let narrow_selection = self.page_delete.is_some() && ctx.screen_rect().width() < 540.0;
        if let Some(selected) = &mut self.page_delete {
            let cells = self
                .note
                .as_ref()
                .map(|n| n.unit_cells())
                .unwrap_or_default();
            selected.retain(|cell| cells.contains(cell));
            if cells.len() <= 1 {
                self.cancel_page_delete();
            }
        }
        if self.page_delete.is_none() {
            self.handle_drops_and_paste(ctx);
        }

        TopBottomPanel::top("rule")
            .exact_height(if narrow_selection { 78.0 } else { 52.0 })
            .show_separator_line(false)
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                let bar = ui.max_rect();
                ctx.data_mut(|d| d.insert_temp(Id::new("top-rect"), bar));
                ui.painter().hline(
                    bar.x_range(),
                    bar.max.y - 0.5,
                    Stroke::new(1.0_f32, self.look.desk_deep),
                );
                if let Some(selected) = &self.page_delete {
                    let count = selected.len();
                    let total = self.note.as_ref().map_or(0, |n| n.unit_cells().len());
                    let message = if count + 1 >= total {
                        "Keep at least one page"
                    } else {
                        "Select pages"
                    };
                    if narrow_selection {
                        ui.label(RichText::new(message).font(self.look.mono(13.0)));
                    }
                    ui.horizontal_centered(|ui| {
                        if self.note_action(ui, "Cancel", 90.0, false).clicked() {
                            self.cancel_page_delete();
                        }
                        if !narrow_selection {
                            ui.label(RichText::new(message).font(self.look.mono(13.0)));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let clicked = ui
                                .add_enabled_ui(count > 0 && count < total, |ui| {
                                    self.note_action(ui, &format!("Delete ({count})"), 112.0, true)
                                        .clicked()
                                })
                                .inner;
                            if clicked {
                                self.delete_selected_pages();
                            }
                        });
                    });
                    return;
                }
                ui.horizontal_centered(|ui| {
                    ui.add_space(6.0);
                    if self.shelf_exit_btn(ui).clicked() {
                        self.close_desk();
                        return;
                    }
                    ui.add_space(8.0);
                    let te = TextEdit::singleline(&mut self.title_buf)
                        .font(self.look.serif(20.0))
                        .desired_width(
                            (ui.available_width() - if compact { 128.0 } else { 322.0 })
                                .clamp(48.0, 280.0),
                        )
                        .frame(false);
                    if ui.add(te).changed() {
                        if let Some(n) = &mut self.note {
                            n.title = self.title_buf.clone();
                            self.mark_dirty();
                        }
                    }
                    let chrome = self.chrome_btn();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(8.0);
                        let more = self.round_well(ui, chrome, self.look.paper, paint_more);
                        more.widget_info(|| {
                            WidgetInfo::labeled(
                                WidgetType::Button,
                                ui.is_enabled(),
                                "Notebook menu",
                            )
                        });
                        ui.add_enabled_ui(
                            self.note.as_ref().is_some_and(|n| n.can_tear_unit()),
                            |ui| {
                                let remove_page = self.round_well(
                                    ui,
                                    chrome,
                                    mix_col(self.look.paper, self.look.rust(), 0.56),
                                    paint_delete_page_icon,
                                );
                                remove_page.widget_info(|| {
                                    WidgetInfo::labeled(
                                        WidgetType::Button,
                                        ui.is_enabled(),
                                        "Delete pages",
                                    )
                                });
                                if remove_page.clicked() {
                                    self.start_page_delete();
                                }
                            },
                        );
                        let join = self
                            .note
                            .as_ref()
                            .map(|n| n.sheet_join)
                            .unwrap_or(SheetJoin::Linked);
                        let mut act = None;
                        Popup::menu(&more)
                            .frame(self.note_popup_frame())
                            .show(|ui| {
                                ui.set_width(
                                    312.0_f32.min(ctx.screen_rect().width() - 32.0).max(240.0),
                                );
                                ui.set_max_height((ctx.screen_rect().height() - 84.0).max(88.0));
                                ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                                ScrollArea::vertical()
                                    .max_height((ctx.screen_rect().height() - 84.0).max(88.0))
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        if self.menu_row(ui, "Open…", "Ctrl+O", false, false) {
                                            act = Some(MoreAct::Open);
                                        }
                                        self.menu_sep(ui);
                                        if compact {
                                            if self.menu_row(ui, "Fit page", "0", false, false) {
                                                act = Some(MoreAct::Fit);
                                            }
                                            if self.menu_row(ui, "Paper", "M", false, false) {
                                                act = Some(MoreAct::Paper);
                                            }
                                            self.menu_sep(ui);
                                        }
                                        if self.menu_row(
                                            ui,
                                            "Linked pages",
                                            "",
                                            join == SheetJoin::Linked,
                                            false,
                                        ) {
                                            act = Some(MoreAct::Linked);
                                        }
                                        if self.menu_row(
                                            ui,
                                            "Separate pages",
                                            "",
                                            join == SheetJoin::Separate,
                                            false,
                                        ) {
                                            act = Some(MoreAct::Separate);
                                        }
                                        self.menu_sep(ui);
                                        if self.menu_row(ui, "Download…", "Ctrl+S", false, false)
                                        {
                                            act = Some(MoreAct::Download);
                                        }
                                        self.menu_sep(ui);
                                        if self.menu_row(ui, "Move to trash", "", false, true) {
                                            act = Some(MoreAct::Trash);
                                        }
                                    });
                            });
                        match act {
                            Some(MoreAct::Open) => self.pending_open = true,
                            Some(MoreAct::Linked) => self.set_sheet_join(SheetJoin::Linked),
                            Some(MoreAct::Separate) => self.set_sheet_join(SheetJoin::Separate),
                            Some(MoreAct::Download) => self.save_for_download(ctx),
                            Some(MoreAct::Trash) => self.trash_open_note(),
                            Some(MoreAct::Fit) => self.fit_to_screen(),
                            Some(MoreAct::Paper) => {
                                if let Some(n) = &mut self.note {
                                    n.paper = n.paper.cycle();
                                    self.mark_dirty();
                                }
                            }
                            None => {}
                        }
                        if compact {
                            return;
                        }
                        if self
                            .round_well(ui, chrome, self.look.accent, paint_fit)
                            .clicked()
                        {
                            self.fit_to_screen();
                        }
                        {
                            let pct = format!("{}%", self.zoom_percent());
                            let near_fit = (self.zoom_percent() - 100).abs() <= 2;
                            let (r, lab) =
                                ui.allocate_exact_size(vec2(56.0, chrome), Sense::click());
                            let (face, fg) = self.paint_note_surface(
                                ui,
                                r.shrink(2.0),
                                &lab,
                                mix_col(self.look.paper, self.look.accent, 0.28),
                                10,
                            );
                            ui.painter().text(
                                face.center(),
                                Align2::CENTER_CENTER,
                                pct,
                                self.look.mono(13.0),
                                if near_fit { fg } else { fg.gamma_multiply(0.7) },
                            );
                            if lab.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                self.fit_to_screen();
                            }
                        }
                        let paper = self.round_well(
                            ui,
                            chrome,
                            self.tool_well_fill(Tool::Highlighter, false),
                            paint_paper_icon,
                        );
                        paper.widget_info(|| {
                            WidgetInfo::labeled(
                                WidgetType::Button,
                                ui.is_enabled(),
                                self.note
                                    .as_ref()
                                    .map(|n| n.paper.label())
                                    .unwrap_or("Paper"),
                            )
                        });
                        if paper.clicked() {
                            if let Some(n) = &mut self.note {
                                n.paper = n.paper.cycle();
                                self.mark_dirty();
                            }
                        }
                    });
                });
            });

        CentralPanel::default()
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                self.ui_canvas(ui);
            });

        if self.page_delete.is_none() {
            self.ui_floating_dock(ctx);
            self.ui_ink_strip(ctx);
            self.ui_ink_tin(ctx);
        }
        if let Some((page, local)) = self.pending_image.take() {
            self.pick_image(page, local);
        }
        self.ui_export_format_picker(ctx);
    }

    fn ui_export_format_picker(&mut self, ctx: &Context) {
        if !self.export_picker_open {
            return;
        }

        let mut export_pdf = false;
        let mut export_png = false;
        let paper = self.paper_tex(ctx);
        let response = Modal::new(Id::new("export-format-picker"))
            .frame(
                self.note_popup_frame()
                    .fill(self.look.paper)
                    .corner_radius(12)
                    .stroke(Stroke::new(1.0, self.look.ink.gamma_multiply(0.18))),
            )
            .show(ctx, |ui| {
                let grain = ui.painter().add(Shape::Noop);
                ui.set_width((ctx.screen_rect().width() - 32.0).clamp(240.0, 360.0));
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Download your note")
                            .font(self.look.serif(21.0))
                            .color(self.look.ink),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("Choose a file format")
                            .font(self.look.mono(12.0))
                            .color(self.look.ink.gamma_multiply(0.72)),
                    );
                    ui.add_space(12.0);
                    let width = (ui.available_width() - 4.0).max(180.0);
                    if self.note_action(ui, "PNG image", width, false).clicked() {
                        export_png = true;
                    }
                    ui.add_space(4.0);
                    if self.note_action(ui, "PDF document", width, false).clicked() {
                        export_pdf = true;
                    }
                    ui.add_space(8.0);
                    if self.note_action(ui, "Cancel", width, false).clicked() {
                        self.export_picker_open = false;
                    }
                    ui.add_space(4.0);
                });
                ui.painter()
                    .set(grain, paper_grain(ui.min_rect().expand(7.0), 11, &paper));
            });

        if response.should_close() {
            self.export_picker_open = false;
        }
        if export_png || export_pdf {
            self.export_picker_open = false;
            if export_png {
                self.export_png(ctx);
            } else {
                self.export_pdf(ctx);
            }
        }
    }

    fn ui_floating_dock(&mut self, ctx: &Context) {
        let bounds = note_controls_bounds(ctx);
        let vertical = self.dock_edge.vertical();
        let edge_key = match self.dock_edge {
            DockEdge::Top => 0_u8,
            DockEdge::Bottom => 1,
            DockEdge::Left => 2,
            DockEdge::Right => 3,
        };
        // Distinct id per edge, so egui does not reuse the previous layout size
        // (a vertical bar would become a large square once horizontal).
        let mut area = Area::new(Id::new(("cahier-dock", edge_key)))
            .order(Order::Foreground)
            .interactable(true)
            .constrain_to(bounds);
        if let Some(pos) = self.dock_float {
            let pos = pos.clamp(bounds.min, bounds.max - vec2(48.0, 48.0).min(bounds.size()));
            area = area.current_pos(pos);
        } else {
            let (align, off) = match self.dock_edge {
                DockEdge::Bottom => (Align2::CENTER_BOTTOM, vec2(0.0, -4.0)),
                DockEdge::Top => (Align2::CENTER_TOP, Vec2::ZERO),
                DockEdge::Left => (Align2::LEFT_CENTER, vec2(2.0, 0.0)),
                DockEdge::Right => (Align2::RIGHT_CENTER, vec2(-2.0, 0.0)),
            };
            area = area.anchor(align, off);
        }
        let inner = area.show(ctx, |ui| {
            // Shrink-wrap, so the area stays inside a tight max_rect.
            ui.set_max_size(bounds.size());
            Frame::NONE
                .fill(self.note_case_color())
                .stroke(Stroke::new(
                    1.0_f32,
                    mix_col(self.note_case_color(), self.look.paper, 0.22),
                ))
                .corner_radius(30)
                .shadow(egui::epaint::Shadow {
                    offset: [0, 4],
                    blur: 8,
                    spread: 0,
                    color: self.look.shadow.gamma_multiply(0.65),
                })
                .inner_margin(if vertical {
                    Margin::symmetric(6, 8)
                } else {
                    Margin::symmetric(8, 6)
                })
                .show(ui, |ui| {
                    if vertical {
                        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                        ui.vertical(|ui| {
                            self.dock_handle(ui, true);
                            ScrollArea::vertical()
                                .id_salt("dock-tools-vertical")
                                .max_height((bounds.height() - 70.0).max(44.0))
                                .max_width(44.0)
                                .auto_shrink([true, true])
                                .show(ui, |ui| {
                                    // The scroll child otherwise inherits the whole window width,
                                    // which shifts the centered tool wells away from their case.
                                    ui.set_width(self.slot());
                                    ui.with_layout(Layout::top_down(Align::Center), |ui| {
                                        self.dock_inner(ui, true);
                                    });
                                });
                        });
                    } else {
                        ui.spacing_mut().item_spacing = vec2(3.0, 0.0);
                        ui.horizontal(|ui| {
                            self.dock_handle(ui, false);
                            ScrollArea::horizontal()
                                .id_salt("dock-tools-horizontal")
                                .max_width((bounds.width() - 70.0).max(44.0))
                                .max_height(44.0)
                                .auto_shrink([true, true])
                                .show(ui, |ui| {
                                    // Keep the horizontal scroller's cross-axis the size of one
                                    // well. Its unconstrained child Ui would otherwise vertically
                                    // center the row in the full available canvas.
                                    ui.set_height(self.slot());
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        self.dock_inner(ui, false);
                                    });
                                });
                        });
                    }
                });
        });
        ctx.data_mut(|d| d.insert_temp(Id::new("dock-rect"), inner.response.rect));
    }

    fn ui_ink_strip(&mut self, ctx: &Context) {
        let bounds = note_controls_bounds(ctx);
        if !self.palette_open
            || (self.tin_open && (bounds.width() < 600.0 || bounds.height() < 500.0))
        {
            ctx.data_mut(|d| d.remove::<Rect>(Id::new("strip-rect")));
            return;
        }
        let screen = ctx.screen_rect();
        let dock = ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap_or(Rect::from_center_size(screen.center(), vec2(48.0, 48.0)));
        let vertical = self.dock_edge.vertical();
        let high = self.tool == Tool::Highlighter;
        let n_colors = if high {
            self.look.highs.len()
        } else {
            self.look.inks.len().saturating_sub(1).max(1)
        };
        let show_w = self.tool.is_ink() || self.tool.is_eraser();
        let long = 14.0
            + n_colors as f32 * if vertical { 30.0 } else { 32.0 }
            + 46.0
            + if show_w { 94.0 } else { 0.0 };
        let wanted = if vertical {
            vec2(58.0, long)
        } else {
            vec2(long, 58.0)
        };
        let place = accessory_rect(bounds, &[dock], wanted, vec2(58.0, 58.0), self.dock_edge);
        let (sw, sh) = (place.width(), place.height());
        let current = Color32::from_rgb(self.ink.r(), self.ink.g(), self.ink.b());
        // Do not reuse the horizontal area's width for a vertical palette.
        let inner = Area::new(Id::new(("cahier-ink-strip", vertical)))
            .order(Order::Foreground)
            .fixed_pos(place.min)
            .constrain_to(bounds)
            .show(ctx, |ui| {
                ui.set_max_size(vec2(sw, sh));
                Frame::NONE
                    .fill(self.note_case_color())
                    .stroke(Stroke::new(
                        1.0_f32,
                        mix_col(self.note_case_color(), self.look.paper, 0.22),
                    ))
                    .corner_radius(22)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 4],
                        blur: 8,
                        spread: 0,
                        color: self.look.shadow.gamma_multiply(0.65),
                    })
                    .inner_margin(Margin::symmetric(6, 6))
                    .show(ui, |ui| {
                        if vertical {
                            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                            ScrollArea::vertical()
                                .id_salt("ink-strip-vertical")
                                .max_height((sh - 14.0).max(44.0))
                                .max_width(44.0)
                                .auto_shrink([true, true])
                                .show(ui, |ui| {
                                    ui.with_layout(Layout::top_down(Align::Center), |ui| {
                                        self.ink_strip_inner(ui, true, high, current, show_w);
                                    });
                                });
                        } else {
                            ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                            ScrollArea::horizontal()
                                .id_salt("ink-strip-horizontal")
                                .max_width((sw - 14.0).max(44.0))
                                .max_height(44.0)
                                .auto_shrink([true, true])
                                .show(ui, |ui| {
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        self.ink_strip_inner(ui, false, high, current, show_w);
                                    });
                                });
                        }
                    });
            });
        ctx.data_mut(|d| d.insert_temp(Id::new("strip-rect"), inner.response.rect));
        if ctx.input(|i| i.pointer.primary_pressed()) {
            if let Some(p) = ctx.pointer_latest_pos() {
                let on_strip = inner.response.rect.expand(4.0).contains(p);
                let on_dock = dock.expand(6.0).contains(p);
                let on_tin = ctx
                    .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
                    .map(|r| r.expand(4.0).contains(p))
                    .unwrap_or(false);
                if !on_strip && !on_dock && !on_tin {
                    self.palette_open = false;
                    self.tin_open = false;
                }
            }
        }
    }

    fn ink_strip_inner(
        &mut self,
        ui: &mut Ui,
        vertical: bool,
        high: bool,
        current: Color32,
        show_w: bool,
    ) {
        if high {
            let n = self.look.highs.len();
            for i in 0..n {
                let c = self.look.highs[i];
                let swatch = Color32::from_rgb(c.r(), c.g(), c.b());
                if self.color_dot(ui, swatch, same_rgb(swatch, current), vertical) {
                    self.pick_theme_ink(i);
                }
            }
        } else {
            let n = self.look.inks.len().saturating_sub(1).max(1);
            for i in 0..n {
                let c = self.look.inks[i];
                if self.color_dot(ui, c, same_rgb(c, current) && !self.color_custom, vertical) {
                    self.pick_theme_ink(i);
                }
            }
        }
        if self.custom_ink_well(ui, self.tin_open).clicked() {
            self.tin_open = !self.tin_open;
        }
        if show_w {
            ui.add_space(4.0);
            for &(w, r) in &[(2.2_f32, 3.0), (5.0, 4.4), (11.0, 6.0)] {
                let on = (self.width - w).abs() < 1.6;
                if self.thick_dot(ui, r, on, vertical) {
                    self.width = w;
                    if self.tool.is_ink() {
                        self.last_ink_width = self.width;
                    }
                }
            }
        }
    }

    fn ui_ink_tin(&mut self, ctx: &Context) {
        if !self.tin_open {
            ctx.data_mut(|d| d.remove::<Rect>(Id::new("tin-rect")));
            return;
        }
        let screen = ctx.screen_rect();
        let dock = ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap_or(Rect::from_center_size(screen.center(), vec2(48.0, 48.0)));
        let bounds = note_controls_bounds(ctx);
        let mut blockers = vec![dock];
        if let Some(strip) = ctx.data(|d| d.get_temp::<Rect>(Id::new("strip-rect"))) {
            blockers.push(strip);
        }
        let place = accessory_rect(
            bounds,
            &blockers,
            vec2(230.0, 382.0),
            vec2(230.0, 100.0),
            self.dock_edge,
        );
        let (tin_w, tin_h) = (place.width(), place.height());
        let paper = self.paper_tex(ctx);
        let inner = Area::new(Id::new("cahier-tin"))
            .order(Order::Foreground)
            .fixed_pos(place.min)
            .constrain_to(bounds)
            .show(ctx, |ui| {
                self.note_popup_frame()
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(tin_w - 26.0);
                        ui.spacing_mut().item_spacing = vec2(4.0, 8.0);
                        ScrollArea::vertical()
                            .max_height(tin_h - 26.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                let sq = 204.0;
                                let (header, _) =
                                    ui.allocate_exact_size(vec2(sq, 28.0), Sense::hover());
                                ui.painter().text(
                                    header.left_center(),
                                    Align2::LEFT_CENTER,
                                    "Ink",
                                    self.look.serif(18.0),
                                    self.look.paper,
                                );
                                let sample = pos2(header.max.x - 12.0, header.center().y);
                                ui.painter().circle_filled(sample, 10.0, self.ink);
                                ui.painter().circle_stroke(
                                    sample,
                                    10.0,
                                    Stroke::new(1.5, self.look.paper),
                                );
                                let (sv, sv_resp) = ui
                                    .allocate_exact_size(vec2(sq, 132.0), Sense::click_and_drag());
                                sv_resp.widget_info(|| {
                                    WidgetInfo::labeled(
                                        WidgetType::Slider,
                                        ui.is_enabled(),
                                        "Ink saturation and brightness",
                                    )
                                });
                                ui.painter().rect_filled(sv.expand(3.0), 6, self.look.paper);
                                paint_sv_field(ui.painter(), sv, self.tin_hue);
                                let (_, s0, v0) = rgb_to_hsv(self.ink);
                                let cur = pos2(
                                    sv.min.x + s0 * sv.width(),
                                    sv.min.y + (1.0 - v0) * sv.height(),
                                );
                                ui.painter().circle_stroke(
                                    cur,
                                    7.0,
                                    Stroke::new(2.0_f32, self.look.paper),
                                );
                                ui.painter().circle_stroke(
                                    cur,
                                    5.0,
                                    Stroke::new(1.0_f32, self.look.ink),
                                );
                                if sv_resp.dragged() || sv_resp.clicked() {
                                    if let Some(p) = sv_resp.interact_pointer_pos() {
                                        let s = ((p.x - sv.min.x) / sv.width()).clamp(0.0, 1.0);
                                        let v =
                                            (1.0 - (p.y - sv.min.y) / sv.height()).clamp(0.0, 1.0);
                                        self.pick_free_ink(hsv_to_rgb(self.tin_hue, s, v));
                                    }
                                }

                                let (hue_hit, hue_resp) =
                                    ui.allocate_exact_size(vec2(sq, 44.0), Sense::click_and_drag());
                                hue_resp.widget_info(|| {
                                    WidgetInfo::labeled(
                                        WidgetType::Slider,
                                        ui.is_enabled(),
                                        "Ink hue",
                                    )
                                });
                                let hue_r =
                                    Rect::from_center_size(hue_hit.center(), vec2(sq, 16.0));
                                ui.painter()
                                    .rect_filled(hue_r.expand(3.0), 6, self.look.paper);
                                paint_hue_bar(ui.painter(), hue_r);
                                let hx = hue_r.min.x + self.tin_hue * hue_r.width();
                                let handle = pos2(hx, hue_r.center().y);
                                ui.painter().circle_filled(
                                    handle,
                                    8.0,
                                    hsv_to_rgb(self.tin_hue, 1.0, 1.0),
                                );
                                ui.painter().circle_stroke(
                                    handle,
                                    8.0,
                                    Stroke::new(2.0, self.look.paper),
                                );
                                ui.painter().circle_stroke(
                                    handle,
                                    9.5,
                                    Stroke::new(1.0, self.look.ink),
                                );
                                if hue_resp.dragged() || hue_resp.clicked() {
                                    if let Some(p) = hue_resp.interact_pointer_pos() {
                                        self.tin_hue =
                                            ((p.x - hue_r.min.x) / hue_r.width()).clamp(0.0, 1.0);
                                        let (_, s, v) = rgb_to_hsv(self.ink);
                                        self.pick_free_ink(hsv_to_rgb(
                                            self.tin_hue,
                                            s.max(0.12),
                                            v.max(0.18),
                                        ));
                                    }
                                }

                                let cols = 12;
                                let rows = 6;
                                let pan = 14.5;
                                let gap_p = 2.6;
                                let grid_w = cols as f32 * pan + (cols - 1) as f32 * gap_p;
                                let grid_h = (rows + 1) as f32 * pan + rows as f32 * gap_p;
                                let (grid, _) = ui.allocate_exact_size(
                                    vec2(grid_w.max(sq), grid_h),
                                    Sense::hover(),
                                );
                                let tray = grid.expand(4.0);
                                ui.painter().rect_filled(tray, 10, self.look.paper);
                                let uv = Rect::from_min_max(
                                    Pos2::ZERO,
                                    pos2(tray.width() / PAPER_TILE, tray.height() / PAPER_TILE),
                                );
                                ui.painter().add(
                                    egui::epaint::RectShape::filled(
                                        tray,
                                        10,
                                        Color32::WHITE.gamma_multiply(0.20),
                                    )
                                    .with_texture(paper.id(), uv),
                                );
                                let origin =
                                    pos2(grid.min.x + (grid.width() - grid_w) * 0.5, grid.min.y);
                                let painter = ui.painter();
                                for row in 0..rows {
                                    for col in 0..cols {
                                        let h = col as f32 / cols as f32;
                                        let v = 0.92 - row as f32 * 0.13;
                                        let s = 0.88;
                                        let c = hsv_to_rgb(h, s, v);
                                        let r = Rect::from_min_size(
                                            origin
                                                + vec2(
                                                    col as f32 * (pan + gap_p),
                                                    row as f32 * (pan + gap_p),
                                                ),
                                            vec2(pan, pan),
                                        );
                                        painter.circle_filled(
                                            r.center() + vec2(0.0, 1.2),
                                            pan * 0.42,
                                            self.look.shadow,
                                        );
                                        painter.circle_filled(r.center(), pan * 0.42, c);
                                        painter.circle_stroke(
                                            r.center(),
                                            pan * 0.42,
                                            Stroke::new(0.7, self.look.ink.gamma_multiply(0.25)),
                                        );
                                        if same_rgb(c, self.ink) {
                                            painter.circle_stroke(
                                                r.center(),
                                                pan * 0.42 + 2.0,
                                                Stroke::new(1.5_f32, self.look.ink),
                                            );
                                        }
                                        let hit = ui.interact(
                                            r,
                                            Id::new(("tin-pan", row, col)),
                                            Sense::click(),
                                        );
                                        hit.widget_info(|| {
                                            WidgetInfo::selected(
                                                WidgetType::Button,
                                                ui.is_enabled(),
                                                same_rgb(c, self.ink),
                                                format!("Ink {} / {}", row + 1, col + 1),
                                            )
                                        });
                                        if hit.has_focus() {
                                            painter.rect_stroke(
                                                r,
                                                3,
                                                Stroke::new(1.5, self.look.ink),
                                                StrokeKind::Inside,
                                            );
                                        }
                                        if hit.gained_focus() {
                                            hit.scroll_to_me(Some(Align::Center));
                                        }
                                        if hit.clicked() {
                                            self.pick_free_ink(c);
                                        }
                                    }
                                }
                                let gray_y = origin.y + rows as f32 * (pan + gap_p);
                                for col in 0..cols {
                                    let g = col as f32 / (cols - 1) as f32;
                                    let c = hsv_to_rgb(0.0, 0.0, g);
                                    let r = Rect::from_min_size(
                                        pos2(origin.x + col as f32 * (pan + gap_p), gray_y),
                                        vec2(pan, pan),
                                    );
                                    painter.circle_filled(
                                        r.center() + vec2(0.0, 1.2),
                                        pan * 0.42,
                                        self.look.shadow,
                                    );
                                    painter.circle_filled(r.center(), pan * 0.42, c);
                                    painter.circle_stroke(
                                        r.center(),
                                        pan * 0.42,
                                        Stroke::new(0.6_f32, self.look.ink.gamma_multiply(0.35)),
                                    );
                                    if same_rgb(c, self.ink) {
                                        painter.circle_stroke(
                                            r.center(),
                                            pan * 0.42 + 2.0,
                                            Stroke::new(1.5_f32, self.look.ink),
                                        );
                                    }
                                    let hit =
                                        ui.interact(r, Id::new(("tin-gray", col)), Sense::click());
                                    hit.widget_info(|| {
                                        WidgetInfo::selected(
                                            WidgetType::Button,
                                            ui.is_enabled(),
                                            same_rgb(c, self.ink),
                                            format!("Gray {}", col + 1),
                                        )
                                    });
                                    if hit.has_focus() {
                                        painter.rect_stroke(
                                            r,
                                            3,
                                            Stroke::new(1.5, self.look.ink),
                                            StrokeKind::Inside,
                                        );
                                    }
                                    if hit.gained_focus() {
                                        hit.scroll_to_me(Some(Align::Center));
                                    }
                                    if hit.clicked() {
                                        self.pick_free_ink(c);
                                    }
                                }
                                ui.add_space(4.0);
                            });
                    });
            });
        ctx.data_mut(|d| d.insert_temp(Id::new("tin-rect"), inner.response.rect));
        if ctx.input(|i| i.pointer.primary_pressed()) {
            if let Some(p) = ctx.pointer_latest_pos() {
                let on_tin = inner.response.rect.expand(4.0).contains(p);
                let on_dock = dock.expand(6.0).contains(p);
                let on_strip = ctx
                    .data(|d| d.get_temp::<Rect>(Id::new("strip-rect")))
                    .map(|r| r.expand(4.0).contains(p))
                    .unwrap_or(false);
                if !on_tin && !on_dock && !on_strip {
                    self.tin_open = false;
                }
            }
        }
    }

    fn dock_handle(&mut self, ui: &mut Ui, vertical: bool) {
        let grip = self.dock_grip(ui, vertical);
        if grip.drag_started() {
            let bar = ui
                .ctx()
                .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
                .unwrap_or(grip.rect);
            let pointer = grip.interact_pointer_pos().unwrap_or(bar.min);
            self.dock_grab = pointer - bar.min;
            self.dock_float = Some(bar.min);
            self.dock_moved = false;
        }
        if grip.dragged() {
            if grip.drag_delta().length() > 0.3 {
                self.dock_moved = true;
            }
            if let Some(p) = grip.interact_pointer_pos() {
                self.dock_float = Some(p - self.dock_grab);
            }
        }
        if grip.drag_stopped() {
            if self.dock_moved {
                let p = grip
                    .interact_pointer_pos()
                    .or(self.dock_float)
                    .unwrap_or_else(|| ui.ctx().screen_rect().center());
                self.dock_edge = snap_dock(p, ui.ctx().screen_rect());
                self.lib.index.dock = self.dock_edge;
                let saved = self.lib.save_index();
                self.report_save(saved);
            }
            self.dock_float = None;
            self.dock_moved = false;
        }
    }

    fn dock_inner(&mut self, ui: &mut Ui, vertical: bool) {
        let slot = self.slot();
        if self
            .round_well(ui, slot, self.look.paper, paint_undo)
            .clicked()
        {
            let mut ok = false;
            if let Some(n) = &mut self.note {
                ok = self.undo.undo(n);
            }
            if ok {
                self.mark_dirty();
            }
        }
        if self
            .round_well(ui, slot, self.look.paper, paint_redo)
            .clicked()
        {
            let mut ok = false;
            if let Some(n) = &mut self.note {
                ok = self.undo.redo(n);
            }
            if ok {
                self.mark_dirty();
            }
        }
        self.dock_gap(ui, vertical);
        for t in [
            Tool::Fineliner,
            Tool::Brush,
            Tool::Pencil,
            Tool::Highlighter,
        ] {
            if self.tool_glyph(ui, t) {
                self.finish_text_edit();
                if self.tool == t {
                    self.width = next_width(self.width);
                    self.last_ink_width = self.width;
                } else {
                    self.tool = t;
                    self.last_ink = t;
                    if let Some(nib) = t.nib() {
                        self.width = default_width(nib);
                        self.last_ink_width = self.width;
                    }
                }
            }
        }
        let pen = self.tablet.snapshot();
        for kind in [Tool::EraserStroke, Tool::EraserArea] {
            let held = pen.eraser && self.last_eraser == kind;
            let on = self.tool == kind || held;
            if self.paint_tool_well(ui, kind, on).clicked() {
                self.finish_text_edit();
                self.pick_eraser(kind);
            }
        }
        let lasso_resp =
            self.paint_tool_well(ui, Tool::Lasso, self.tool == Tool::Lasso || pen.lasso_btn);
        if lasso_resp.clicked() {
            self.finish_text_edit();
            self.tool = Tool::Lasso;
        }
        self.dock_gap(ui, vertical);
        let current = Color32::from_rgb(self.ink.r(), self.ink.g(), self.ink.b());
        if self
            .palette_chip(ui, current, self.palette_open || self.tin_open, vertical)
            .clicked()
        {
            self.palette_open = !self.palette_open;
            if !self.palette_open {
                self.tin_open = false;
            }
        }
        self.dock_gap(ui, vertical);
        if self.tool_glyph(ui, Tool::Text) {
            self.tool = Tool::Text;
        }
        if self.tool_glyph(ui, Tool::Image) {
            self.finish_text_edit();
            self.tool = Tool::Image;
            self.pending_image = Some(self.drop_spot());
        }
    }

    fn dock_grip(&self, ui: &mut Ui, vertical: bool) -> Response {
        let s = self.slot();
        let (rect, resp) = ui.allocate_exact_size(vec2(s, s), Sense::click_and_drag());
        resp.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Move toolbar")
        });
        let (face, fg) = self.paint_note_surface(
            ui,
            rect.shrink(2.0),
            &resp,
            self.look.paper,
            (s * 0.5) as u8,
        );
        let p = ui.painter();
        let c = face.center();
        let fg = fg.gamma_multiply(0.55);
        if vertical {
            for i in 0..3 {
                let x = c.x - 6.0 + i as f32 * 6.0;
                p.circle_filled(pos2(x, c.y - 2.4), 1.55, fg);
                p.circle_filled(pos2(x, c.y + 2.4), 1.55, fg);
            }
        } else {
            for i in 0..3 {
                let y = c.y - 6.0 + i as f32 * 6.0;
                p.circle_filled(pos2(c.x - 2.4, y), 1.55, fg);
                p.circle_filled(pos2(c.x + 2.4, y), 1.55, fg);
            }
        }
        let cursor = if resp.dragged() {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        };
        resp.on_hover_cursor(cursor)
    }

    fn dock_gap(&self, ui: &mut Ui, vertical: bool) {
        let s = self.slot();
        if vertical {
            ui.add_space(2.0);
            let (rect, _) = ui.allocate_exact_size(vec2(s, 2.0), Sense::hover());
            ui.painter().line_segment(
                [
                    pos2(rect.min.x + 1.0, rect.center().y),
                    pos2(rect.max.x - 1.0, rect.center().y),
                ],
                Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.55)),
            );
            ui.add_space(2.0);
        } else {
            ui.add_space(2.0);
            let (rect, _) = ui.allocate_exact_size(vec2(2.0, s), Sense::hover());
            ui.painter().line_segment(
                [
                    pos2(rect.center().x, rect.min.y + 1.0),
                    pos2(rect.center().x, rect.max.y - 1.0),
                ],
                Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.55)),
            );
            ui.add_space(2.0);
        }
    }

    fn palette_chip(&self, ui: &mut Ui, color: Color32, open: bool, vertical: bool) -> Response {
        let s = self.slot();
        let size = if vertical {
            vec2(s, 30.0)
        } else {
            vec2(32.0, s)
        };
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let c = rect.center();
        let p = ui.painter();
        let r = 7.4;
        if !open {
            p.circle_filled(c + vec2(2.4, 2.1), r - 0.6, color.gamma_multiply(0.55));
        }
        p.circle_filled(c, r + 1.3, self.look.fg.gamma_multiply(0.55));
        p.circle_filled(c, r, color);
        p.circle_stroke(
            c,
            r,
            Stroke::new(1.05_f32, self.look.fg.gamma_multiply(0.55)),
        );
        if open {
            p.circle_stroke(c, r + 3.4, Stroke::new(1.5_f32, self.look.fg));
        }
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn custom_ink_well(&self, ui: &mut Ui, on: bool) -> Response {
        let s = self.slot();
        let resp = self.round_well(ui, s, self.look.paper, |p, c, fg| {
            self.paint_palette_icon(p, c, fg)
        });
        if on {
            ui.painter().rect_stroke(
                resp.rect.shrink(0.5),
                CornerRadius::same(22),
                Stroke::new(2.0, self.look.paper),
                StrokeKind::Inside,
            );
        }
        resp.widget_info(|| {
            WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), on, "Custom ink")
        });
        resp
    }

    fn paint_palette_icon(&self, p: &Painter, c: Pos2, fg: Color32) {
        // Three paint wells: the same pigments as the shelf, not an OS emoji.
        for (offset, color) in [
            (vec2(-6.0, -4.0), self.look.rust()),
            (vec2(6.0, -4.0), self.look.green()),
            (vec2(0.0, 7.0), self.look.accent),
        ] {
            p.circle_filled(c + offset, 4.8, color);
            p.circle_stroke(c + offset, 4.8, Stroke::new(1.0, fg.gamma_multiply(0.65)));
        }
    }

    fn color_dot(&self, ui: &mut Ui, color: Color32, on: bool, vertical: bool) -> bool {
        let s = self.slot();
        let size = if vertical {
            vec2(s, 28.0)
        } else {
            vec2(30.0, s)
        };
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let c = rect.center();
        let p = ui.painter();
        let r = if on { 9.4 } else { 8.0 };
        p.circle_filled(c, r + 1.4, self.look.fg.gamma_multiply(0.55));
        p.circle_filled(c, r, color);
        p.circle_stroke(
            c,
            r,
            Stroke::new(1.05_f32, self.look.fg.gamma_multiply(0.55)),
        );
        if on {
            p.circle_stroke(c, r + 3.8, Stroke::new(1.7_f32, self.look.fg));
        }
        resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
    }

    fn thick_dot(&self, ui: &mut Ui, r: f32, on: bool, vertical: bool) -> bool {
        let s = self.slot();
        let size = if vertical {
            vec2(s, 28.0)
        } else {
            vec2(28.0, s)
        };
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let c = rect.center();
        let p = ui.painter();
        let fill = if on {
            self.ink_color()
        } else {
            self.look.fg.gamma_multiply(0.42)
        };
        p.circle_filled(c, r, fill);
        resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
    }

    fn tool_glyph(&mut self, ui: &mut Ui, tool: Tool) -> bool {
        self.paint_tool_well(ui, tool, self.tool == tool).clicked()
    }

    fn paint_tool_well(&mut self, ui: &mut Ui, tool: Tool, active: bool) -> Response {
        let resp = self.round_well(
            ui,
            self.slot(),
            self.tool_well_fill(tool, active),
            |p, c, fg| {
                let stroke = Stroke::new(1.7, fg);
                match tool {
                    Tool::Text => paint_text_icon(p, c, fg),
                    Tool::Image => paint_image_icon(p, c, fg),
                    Tool::Lasso => {
                        p.circle_stroke(c + vec2(0.0, -3.0), 8.0, stroke);
                        p.add(Shape::line(
                            vec![
                                c + vec2(0.0, 5.0),
                                c + vec2(4.0, 10.0),
                                c + vec2(-2.0, 13.0),
                            ],
                            stroke,
                        ));
                    }
                    Tool::EraserStroke | Tool::EraserArea => {
                        p.add(Shape::closed_line(
                            vec![
                                c + vec2(-11.0, 3.0),
                                c + vec2(-2.0, -9.0),
                                c + vec2(11.0, 0.0),
                                c + vec2(2.0, 11.0),
                                c + vec2(-1.0, 11.0),
                            ],
                            stroke,
                        ));
                        p.line_segment([c + vec2(-6.0, -3.0), c + vec2(7.0, 5.0)], stroke);
                        if tool == Tool::EraserArea {
                            for x in [-7.0, 0.0, 7.0] {
                                p.circle_filled(c + vec2(x, 14.0), 1.1, fg);
                            }
                        } else {
                            p.line_segment([c + vec2(-12.0, 14.0), c + vec2(12.0, 14.0)], stroke);
                        }
                    }
                    Tool::Fineliner => {
                        p.add(Shape::closed_line(
                            vec![
                                c + vec2(0.0, -13.0),
                                c + vec2(-9.0, 3.0),
                                c + vec2(-5.0, 11.0),
                                c + vec2(5.0, 11.0),
                                c + vec2(9.0, 3.0),
                            ],
                            stroke,
                        ));
                        p.line_segment([c + vec2(0.0, -12.0), c + vec2(0.0, 2.0)], stroke);
                        p.circle_filled(c + vec2(0.0, 3.0), 2.0, fg);
                    }
                    Tool::Brush => {
                        p.add(Shape::closed_line(
                            vec![
                                c + vec2(-2.0, 2.0),
                                c + vec2(5.0, -13.0),
                                c + vec2(8.0, -12.0),
                                c + vec2(3.0, 4.0),
                            ],
                            stroke,
                        ));
                        p.add(Shape::convex_polygon(
                            vec![
                                c + vec2(-2.0, 3.0),
                                c + vec2(3.0, 5.0),
                                c + vec2(1.0, 11.0),
                                c + vec2(-9.0, 13.0),
                                c + vec2(-5.0, 8.0),
                            ],
                            fg,
                            Stroke::NONE,
                        ));
                    }
                    Tool::Pencil | Tool::Highlighter => {
                        let w = if tool == Tool::Highlighter { 7.0 } else { 4.0 };
                        p.add(Shape::closed_line(
                            vec![
                                c + vec2(-w, -12.0),
                                c + vec2(w, -12.0),
                                c + vec2(w, 5.0),
                                c + vec2(0.0, 13.0),
                                c + vec2(-w, 5.0),
                            ],
                            stroke,
                        ));
                        p.line_segment([c + vec2(-w, 4.0), c + vec2(w, 4.0)], stroke);
                        if tool == Tool::Highlighter {
                            p.line_segment(
                                [c + vec2(-8.0, 15.0), c + vec2(8.0, 15.0)],
                                Stroke::new(3.0, fg),
                            );
                        } else {
                            p.line_segment([c + vec2(0.0, -9.0), c + vec2(0.0, 2.0)], stroke);
                        }
                    }
                }
            },
        );
        if active {
            // Selection stays legible without relying on the pigment alone.
            ui.painter().rect_stroke(
                resp.rect.shrink(0.5),
                CornerRadius::same(22),
                Stroke::new(2.0, self.look.paper),
                StrokeKind::Inside,
            );
        }
        resp.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::SelectableLabel,
                ui.is_enabled(),
                active,
                tool.label(),
            )
        });
        resp
    }

    fn ui_canvas(&mut self, ui: &mut Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        if !self.update_canvas_rect(rect) {
            return;
        }

        self.handle_camera(ui, &resp, rect);
        if self.page_delete.is_none() {
            self.handle_tool(ui, &resp, rect);
        }
        self.paint_world(&painter, rect);
        if self.page_delete.is_some() {
            self.paint_page_selection(ui, &painter, rect);
            return;
        }
        self.paint_sheet_tabs(ui, &painter, rect);
        self.paint_overlays(ui, rect);
        self.cursor_for_tool(ui, &resp);
    }

    fn update_canvas_rect(&mut self, rect: Rect) -> bool {
        if !Camera::valid_viewport(rect) {
            return false;
        }
        let previous = self.canvas_rect;
        let reference = if Camera::valid_viewport(previous) {
            previous
        } else {
            rect
        };
        let (pw, ph) = self
            .note
            .as_ref()
            .map(|n| n.page_size())
            .unwrap_or((PAGE_W, PAGE_H));
        if let Some(page) = self.land_page.take() {
            let last = self
                .note
                .as_ref()
                .map_or(0, |n| n.pages.len().saturating_sub(1));
            self.camera
                .show_writing(rect, self.origin_of(page.min(last)), pw, ph);
            self.fitted_cell = None;
            self.need_fit = false;
        } else if self.need_fit {
            let page = self.page_in_view(reference);
            self.camera.fit_page(rect, self.origin_of(page), pw, ph);
            self.fitted_cell = self.page_cells().get(page).copied();
            self.need_fit = false;
        } else if previous != rect {
            let fitted = self.fitted_cell.map(|cell| {
                let cells = self.page_cells();
                let cell = if cells.contains(&cell) {
                    cell
                } else {
                    cells
                        .get(self.page_in_view(reference))
                        .copied()
                        .unwrap_or((0, 0))
                };
                self.fitted_cell = Some(cell);
                (
                    page_origin(cell.0, cell.1, pw, ph, self.gap()),
                    vec2(pw, ph),
                )
            });
            self.camera.resize_viewport(previous, rect, fitted);
        }
        self.canvas_rect = rect;
        true
    }

    fn handle_camera(&mut self, ui: &Ui, resp: &Response, rect: Rect) {
        if self.export_picker_open
            || Popup::is_any_open(ui.ctx())
            || ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|p| Self::pen_over_chrome(ui.ctx(), p))
        {
            self.two_finger = None;
            return;
        }
        let before = self.camera;
        let space = ui.input(|i| i.key_down(Key::Space));
        let middle = ui.input(|i| i.pointer.middle_down());
        let pen = self.tablet.snapshot();
        let (egui_zoom, pinch_center, ctrl, point_scroll, line_scroll, mt_pan) = ui.input(|i| {
            let mt = i.multi_touch();
            let mut point = Vec2::ZERO;
            let mut line = Vec2::ZERO;
            for e in &i.events {
                if let Event::MouseWheel { unit, delta, .. } = e {
                    match unit {
                        MouseWheelUnit::Point => point += *delta,
                        MouseWheelUnit::Line | MouseWheelUnit::Page => line += *delta,
                    }
                }
            }
            (
                i.zoom_delta(),
                mt.map(|m| m.center_pos),
                i.modifiers.command,
                point,
                line,
                mt.map(|m| m.translation_delta).unwrap_or(Vec2::ZERO),
            )
        });
        let focus = pinch_center.or(resp.hover_pos()).unwrap_or(rect.center());

        if pen.pinching || (pen.pinch_zoom - 1.0).abs() > 0.0005 {
            self.camera.zoom_at(focus, rect, pen.pinch_zoom);
            self.camera.pan += pen.pinch_pan;
        } else if (egui_zoom - 1.0).abs() > 0.0005 {
            self.camera.zoom_at(focus, rect, egui_zoom);
        } else if !ctrl && point_scroll != Vec2::ZERO {
            // Trackpad: two fingers. Vertical = zoom, horizontal = pan.
            if point_scroll.y.abs() >= point_scroll.x.abs() * 0.35 {
                let f = (1.0 + point_scroll.y * 0.004).clamp(0.55, 1.85);
                self.camera.zoom_at(focus, rect, f);
            } else {
                self.camera.pan += point_scroll;
            }
        } else if mt_pan != Vec2::ZERO {
            self.camera.pan += mt_pan;
        } else if !ctrl && resp.hovered() && line_scroll != Vec2::ZERO {
            self.camera.pan += line_scroll;
        }

        let pan = space || middle;
        if pan && resp.dragged() {
            self.camera.pan += resp.drag_delta();
        }

        let time = ui.input(|i| i.time);
        self.note_touches(ui);
        let mt = ui.input(|i| i.multi_touch());
        if let Some(m) = mt {
            if m.num_touches >= 2 {
                let g = self.two_finger.get_or_insert((time, 0.0, 0.0));
                g.1 += m.translation_delta.length();
                g.2 += (m.zoom_delta - 1.0).abs();
            }
        } else if let Some((t0, travel, zdev)) = self.two_finger.take() {
            if self.is_tablette() && time - t0 < 0.22 && travel < 14.0 && zdev < 0.05 {
                if let Some(n) = &mut self.note {
                    if self.undo.undo(n) {
                        self.mark_dirty();
                    }
                }
            }
        }

        let pen_busy = pen.down || pen.pressed || (self.live.is_some() && self.live_from_pen);
        let finger_pan = if self.is_tablette() {
            self.finger_alive(ui)
        } else {
            ui.input(|i| i.any_touches())
        };
        let on_tab = self.tab_held
            || ui
                .input(|i| i.pointer.press_origin())
                .is_some_and(|p| self.sheet_tab_at(p, rect).is_some());
        if finger_pan
            && !on_tab
            && mt.is_none()
            && !pen.pinching
            && !pen_busy
            && !self.palm_guard(&pen)
            && !pan
            && resp.dragged()
        {
            self.camera.pan += resp.drag_delta();
        }
        if self.camera.pan != before.pan || self.camera.zoom != before.zoom {
            self.fitted_cell = None;
        }
    }

    fn handle_tool(&mut self, ui: &Ui, resp: &Response, rect: Rect) {
        if self.export_picker_open {
            return;
        }
        if self.tab_held {
            let down = ui.input(|i| i.pointer.primary_down()) || self.tablet.snapshot().down;
            self.tab_held = false;
            if down {
                self.tab_held = true;
            }
            return;
        }
        let space = ui.input(|i| i.key_down(Key::Space));
        if space
            || ui.input(|i| i.pointer.middle_down())
            || ui.input(|i| i.multi_touch().is_some())
            || self.tablet.snapshot().pinching
        {
            return;
        }
        let pen = self.tablet.snapshot();
        let pen_ink =
            pen.down || pen.pressed || pen.released || (self.live.is_some() && self.live_from_pen);
        let screen_touch = ui.input(|i| i.any_touches());
        let on_tab = ui
            .input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()))
            .is_some_and(|p| self.sheet_tab_at(p, rect).is_some());
        if !pen_ink && self.live.is_none() && !on_tab {
            if self.is_tablette() {
                if self.palm_guard(&pen) || self.finger_alive(ui) {
                    return;
                }
            } else if screen_touch {
                return;
            }
        }

        let extra: Vec<Pos2>;
        let (screen, primary_down, primary_pressed, primary_released, secondary) = if pen_ink {
            if let Some(screen) = pen.pos {
                extra = pen.samples;
                (screen, pen.down, pen.pressed, pen.released, pen.eraser)
            } else {
                if self.live.is_some() && self.live_from_pen && (pen.released || !pen.down) {
                    self.finish_live();
                }
                return;
            }
        } else {
            let pos = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
            let Some(screen) = pos else {
                return;
            };
            extra = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        Event::PointerMoved(sp) => Some(*sp),
                        Event::Touch {
                            phase: TouchPhase::Move | TouchPhase::Start,
                            pos,
                            ..
                        } => Some(*pos),
                        _ => None,
                    })
                    .collect()
            });
            (
                screen,
                ui.input(|i| i.pointer.primary_down()),
                ui.input(|i| i.pointer.primary_pressed()),
                ui.input(|i| i.pointer.primary_released()),
                ui.input(|i| i.pointer.secondary_down()),
            )
        };

        if !rect.contains(screen) && !(self.live.is_some() && (primary_released || !primary_down)) {
            return;
        }
        if ui.ctx().data(|d| {
            d.get_temp::<Rect>(Id::new("dock-rect"))
                .map(|r| r.expand(6.0).contains(screen))
                .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("top-rect"))
                    .map(|r| r.expand(2.0).contains(screen))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("tin-rect"))
                    .map(|r| r.expand(2.0).contains(screen))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("strip-rect"))
                    .map(|r| r.expand(2.0).contains(screen))
                    .unwrap_or(false)
        }) {
            if self.live.is_some() && (primary_released || !primary_down) {
                self.finish_live();
            }
            return;
        }
        if primary_pressed {
            if let Some((col, row)) = self.sheet_tab_at(screen, rect) {
                self.add_sheet_from_tab(col, row);
                self.tab_held = true;
                return;
            }
        }
        let paper = self.camera.to_paper(screen, rect);
        let page = self.page_at_paper(paper);
        let local = paper - self.origin_of(page);

        let mut tool = self.tool;
        if pen.lasso_btn {
            tool = Tool::Lasso;
        } else if pen.eraser || secondary {
            tool = self.last_eraser;
        }

        if tool.is_ink()
            && (primary_down || primary_pressed)
            && !resp.dragged_by(PointerButton::Middle)
        {
            if self.live.is_none() {
                if let Some(nib) = tool.nib() {
                    self.push_snapshot();
                    let mut s = InkStroke::new(nib, self.ink_color(), self.width);
                    s.push(InkPoint::new(local, 0.7));
                    self.live = Some((page, s));
                    self.live_from_pen = pen_ink;
                    self.shape_anchor = Some(local);
                    self.shape_still = Some(Instant::now());
                    self.shape_preview = None;
                }
            }
            let live_local = self
                .live
                .as_ref()
                .map(|(start_page, _)| paper - self.origin_of(*start_page))
                .unwrap_or(local);
            let speed = self.speed(live_local);
            let hw = self.pressure.latest();
            let linked = self.gap() == 0.0;
            if let Some((p, s)) = &mut self.live {
                if *p == page || linked {
                    let pr = mixed_pressure(s.nib, hw, speed);
                    s.push(InkPoint::new(live_local, pr));
                }
            }
            let cam = self.camera;
            let cells = self.page_cells();
            let pw = self.pw();
            let ph = self.ph();
            let gap = self.gap();
            let pressure = self.pressure.latest();
            if let Some((pgi, live)) = &mut self.live {
                for sp in &extra {
                    let pp = cam.to_paper(*sp, rect);
                    if gap == 0.0 || page_at(pp, &cells, pw, ph, gap) == *pgi {
                        let (col, row) = cells.get(*pgi).copied().unwrap_or((0, 0));
                        let loc = pp - page_origin(col, row, pw, ph, gap);
                        live.push(InkPoint::new(
                            loc,
                            mixed_pressure(live.nib, pressure, 200.0),
                        ));
                    }
                }
            }
            self.consider_shape(live_local);
        }
        if self.live.is_some()
            && (primary_released || !primary_down)
            && !(tool.is_ink() && primary_down)
        {
            self.finish_live();
        }

        if tool.is_eraser() && (primary_down || secondary) {
            if primary_pressed
                || ui.input(|i| i.pointer.secondary_pressed())
                || (pen_ink && pen.pressed && secondary)
            {
                self.push_snapshot();
            }
            let r = if tool == Tool::EraserArea {
                self.width.max(10.0) * 1.8
            } else {
                self.width.max(8.0)
            };
            let mut erased = false;
            if let Some(n) = &mut self.note {
                if let Some(page) = n.pages.get_mut(page) {
                    if tool == Tool::EraserStroke {
                        let hit: Vec<Uuid> = page
                            .strokes
                            .iter()
                            .filter(|s| s.hits(local, r))
                            .map(|s| s.id)
                            .collect();
                        if !hit.is_empty() {
                            page.strokes.retain(|s| !hit.contains(&s.id));
                            erased = true;
                        }
                    } else {
                        let mut neu = Vec::new();
                        let mut changed = false;
                        for s in page.strokes.drain(..) {
                            let parts = erase_area(&s, local, r);
                            if parts.len() != 1 || parts[0].points.len() != s.points.len() {
                                changed = true;
                            }
                            neu.extend(parts);
                        }
                        page.strokes = neu;
                        erased = changed;
                    }
                }
            }
            if erased {
                self.dirty = true;
                self.last_change = Instant::now();
            }
        }

        if tool == Tool::Lasso {
            if primary_pressed {
                if self.sel_contains(paper) {
                    self.push_snapshot();
                    self.drag_last = Some(paper);
                    self.lasso.clear();
                } else {
                    self.lasso.clear();
                    self.sel.clear();
                    self.lasso.push(paper);
                    self.drag_last = None;
                }
            } else if primary_down {
                if let Some(last) = self.drag_last {
                    let d = paper - last;
                    self.translate_sel(d);
                    self.drag_last = Some(paper);
                    self.mark_dirty();
                } else if self
                    .lasso
                    .last()
                    .map(|p| p.distance(paper) > 2.0)
                    .unwrap_or(true)
                {
                    self.lasso.push(paper);
                }
            } else if primary_released {
                self.drag_last = None;
                if self.lasso.len() > 2 {
                    self.sel = self.hits_lasso();
                    self.lasso.clear();
                }
            }
        } else if !primary_down {
            self.drag_last = None;
        }

        let tap = primary_pressed || resp.clicked();
        if tool == Tool::Text && tap {
            if let Some((pg, id)) = self.hit_text(page, local) {
                if self.editing_text != Some((pg, id)) {
                    self.finish_text_edit();
                    self.editing_text = Some((pg, id));
                    self.text_focus = true;
                }
            } else {
                let on_live = self.editing_text.is_some_and(|(pg, id)| {
                    pg == page
                        && self.note.as_ref().is_some_and(|n| {
                            n.pages.get(pg).is_some_and(|p| {
                                p.texts
                                    .iter()
                                    .find(|t| t.id == id)
                                    .is_some_and(|t| t.rect().expand(12.0).contains(local))
                            })
                        })
                });
                if !on_live {
                    self.finish_text_edit();
                    self.push_snapshot();
                    let mut tx = TextBox::new(local, self.ink_color());
                    tx.size = [420.0, 48.0];
                    let id = tx.id;
                    if let Some(n) = &mut self.note {
                        if let Some(pg) = n.pages.get_mut(page) {
                            pg.texts.push(tx);
                        }
                    }
                    self.editing_text = Some((page, id));
                    self.text_focus = true;
                    self.mark_dirty();
                }
            }
        }

        if tool == Tool::Image && tap {
            self.pending_image = Some((page, local));
        }
    }

    fn clear_shape_hold(&mut self) {
        self.shape_anchor = None;
        self.shape_still = None;
        self.shape_preview = None;
    }

    /// After one second still, the free stroke snaps to a line, square, or circle.
    fn consider_shape(&mut self, tip: Pos2) {
        if self.live.is_none() {
            return;
        }
        let still_r = 14.0 / self.camera.zoom.max(0.15);
        let anchor = self.shape_anchor.unwrap_or(tip);
        if anchor.distance(tip) > still_r {
            self.shape_anchor = Some(tip);
            self.shape_still = Some(Instant::now());
            self.shape_preview = None;
            return;
        }
        if self.shape_preview.is_some() {
            return;
        }
        let since = *self.shape_still.get_or_insert_with(Instant::now);
        if since.elapsed() < Duration::from_millis(1000) {
            return;
        }
        let Some((_, stroke)) = &self.live else {
            return;
        };
        self.shape_preview = maybe_snap_shape(stroke);
    }

    fn finish_live(&mut self) {
        self.live_from_pen = false;
        let preview = self.shape_preview.take();
        self.shape_anchor = None;
        self.shape_still = None;
        if let Some((p, mut s)) = self.live.take() {
            if let Some(snapped) = preview {
                s = snapped;
            }
            if !s.points.is_empty() {
                if let Some(n) = &mut self.note {
                    if let Some(page) = n.pages.get_mut(p) {
                        page.strokes.push(s);
                    }
                }
                self.mark_dirty();
            }
        }
    }

    fn speed(&mut self, local: Pos2) -> f32 {
        let now = Instant::now();
        let speed = if let Some((prev, t)) = self.last_ptr {
            let dt = now.saturating_duration_since(t).as_secs_f32().max(1e-3);
            prev.distance(local) / dt
        } else {
            200.0
        };
        self.last_ptr = Some((local, now));
        speed
    }

    fn sel_contains(&self, paper: Pos2) -> bool {
        let Some(n) = &self.note else {
            return false;
        };
        for s in &self.sel {
            let Some(page) = n.pages.get(s.page) else {
                continue;
            };
            let o = self.origin_of(s.page);
            let hit = match s.kind {
                SelKind::Stroke => page
                    .strokes
                    .iter()
                    .find(|x| x.id == s.id)
                    .and_then(|st| st.bbox())
                    .map(|(a, b)| {
                        egui::Rect::from_min_max(a + o, b + o)
                            .expand(12.0)
                            .contains(paper)
                    })
                    .unwrap_or(false),
                SelKind::Text => page
                    .texts
                    .iter()
                    .find(|x| x.id == s.id)
                    .map(|t| t.rect().translate(o).contains(paper))
                    .unwrap_or(false),
                SelKind::Image => page
                    .images
                    .iter()
                    .find(|x| x.id == s.id)
                    .map(|t| t.rect().translate(o).contains(paper))
                    .unwrap_or(false),
            };
            if hit {
                return true;
            }
        }
        false
    }

    fn hits_lasso(&self) -> Vec<Sel> {
        let poly = &self.lasso;
        let mut out = Vec::new();
        let Some(n) = &self.note else {
            return out;
        };
        for (pi, page) in n.pages.iter().enumerate() {
            let o = page_origin(
                page.col,
                page.row,
                n.page_w.max(1.0),
                n.page_h.max(1.0),
                n.sheet_join.gap(),
            );
            for s in &page.strokes {
                let world: Vec<Pos2> = s.points.iter().map(|p| p.pos() + o).collect();
                if world.iter().any(|p| crate::ink::point_in_poly(*p, poly)) {
                    out.push(Sel {
                        page: pi,
                        id: s.id,
                        kind: SelKind::Stroke,
                    });
                }
            }
            for t in &page.texts {
                if crate::ink::point_in_poly(t.min() + o, poly) {
                    out.push(Sel {
                        page: pi,
                        id: t.id,
                        kind: SelKind::Text,
                    });
                }
            }
            for im in &page.images {
                if crate::ink::point_in_poly(im.min() + o, poly) {
                    out.push(Sel {
                        page: pi,
                        id: im.id,
                        kind: SelKind::Image,
                    });
                }
            }
        }
        out
    }

    fn translate_sel(&mut self, d: Vec2) {
        let sel = self.sel.clone();
        if let Some(n) = &mut self.note {
            for s in sel {
                if let Some(page) = n.pages.get_mut(s.page) {
                    match s.kind {
                        SelKind::Stroke => {
                            if let Some(st) = page.strokes.iter_mut().find(|x| x.id == s.id) {
                                st.translate(d);
                            }
                        }
                        SelKind::Text => {
                            if let Some(t) = page.texts.iter_mut().find(|x| x.id == s.id) {
                                t.translate(d);
                            }
                        }
                        SelKind::Image => {
                            if let Some(im) = page.images.iter_mut().find(|x| x.id == s.id) {
                                im.translate(d);
                            }
                        }
                    }
                }
            }
        }
    }

    fn hit_text(&self, page: usize, local: Pos2) -> Option<(usize, Uuid)> {
        let n = self.note.as_ref()?;
        let pg = n.pages.get(page)?;
        pg.texts
            .iter()
            .rev()
            .find(|t| t.contains(local))
            .map(|t| (page, t.id))
    }

    fn pick_image(&mut self, page: usize, local: Pos2) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Place an image")
            .add_filter(
                "Images",
                &["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg"],
            )
            .pick_file()
        else {
            return;
        };
        if crate::import::is_placeable_image(&path) {
            if let Ok(png) = crate::import::load_placed_image(&path) {
                self.insert_image_bytes(&png, page, local);
            }
            return;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        self.insert_image_bytes(&bytes, page, local);
    }

    fn insert_image_bytes(&mut self, bytes: &[u8], page: usize, local: Pos2) {
        let Some(png) = ensure_png(bytes) else {
            return;
        };
        let Some(note) = &self.note else {
            return;
        };
        let id = note.id;
        let Some(file) = self.lib.write_media(id, &png) else {
            return;
        };
        let path = self.lib.media_path(id, &file);
        let (w, h) = image_size(&path).unwrap_or((400.0, 300.0));
        let scale = (420.0 / w.max(h)).min(1.0);
        self.push_snapshot();
        if let Some(n) = &mut self.note {
            if let Some(pg) = n.pages.get_mut(page) {
                pg.images.push(ImageObj {
                    id: Uuid::new_v4(),
                    pos: [local.x, local.y],
                    size: [w * scale, h * scale],
                    file,
                });
            }
        }
        self.mark_dirty();
        self.textures.clear();
    }

    fn handle_drops_and_paste(&mut self, ctx: &Context) {
        let dropped: Vec<(Option<PathBuf>, Vec<u8>)> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| {
                    if let Some(path) = &f.path {
                        Some((Some(path.clone()), Vec::new()))
                    } else {
                        f.bytes.as_ref().map(|bytes| (None, bytes.to_vec()))
                    }
                })
                .collect()
        });
        if !dropped.is_empty() {
            let mut paths = Vec::new();
            let mut loose = Vec::new();
            for (path, bytes) in dropped {
                if let Some(path) = path {
                    paths.push(path);
                } else if bytes.starts_with(b"%PDF") {
                    let path =
                        std::env::temp_dir().join(format!("cahier-drop-{}.pdf", Uuid::new_v4()));
                    if std::fs::write(&path, &bytes).is_ok() {
                        paths.push(path);
                    }
                } else if matches!(self.scene, Scene::Desk) {
                    loose.push(bytes);
                }
            }
            if !paths.is_empty() {
                if let Some(msg) = self.open_paths(&paths) {
                    self.toast(msg, ctx.input(|i| i.time));
                }
            }
            if let Scene::Desk = self.scene {
                for (i, bytes) in loose.iter().enumerate() {
                    let (page, local) = self.drop_spot();
                    let local = local + vec2(i as f32 * 24.0, i as f32 * 24.0);
                    self.insert_image_bytes(bytes, page, local);
                }
            }
        }
        let paste_img = ctx.input(|i| i.modifiers.command && i.key_pressed(Key::V));
        if paste_img {
            if let Ok(mut clip) = arboard::Clipboard::new() {
                if let Ok(img) = clip.get_image() {
                    let mut rgba = Vec::with_capacity(img.bytes.len());
                    rgba.extend_from_slice(&img.bytes);
                    let mut png = Vec::new();
                    if image::RgbaImage::from_raw(img.width as u32, img.height as u32, rgba)
                        .and_then(|im| {
                            image::DynamicImage::ImageRgba8(im)
                                .write_to(
                                    &mut std::io::Cursor::new(&mut png),
                                    image::ImageFormat::Png,
                                )
                                .ok()
                        })
                        .is_some()
                    {
                        if let Scene::Desk = self.scene {
                            self.insert_image_bytes(&png, 0, pos2(90.0, 90.0));
                        }
                    }
                }
            }
        }
    }

    fn paint_world(&mut self, painter: &Painter, rect: Rect) {
        let Some(note) = self.note.clone() else {
            return;
        };
        let canson = if note.paper == PaperKind::Slate {
            None
        } else {
            Some(self.paper_tex(painter.ctx()))
        };
        let cam = self.camera;
        let map = |p: Pos2| cam.to_screen(p, rect);
        let fused =
            note.sheet_join == SheetJoin::Linked && note.pages.len() > 1 && !note.is_grown_single();
        let min_col = note.pages.iter().map(|p| p.col).min().unwrap_or(0);
        let fill = if note.paper == PaperKind::Slate {
            self.look.desk_deep
        } else {
            self.look.paper
        };
        let bleed = 1.6;
        let occupied: HashSet<_> = note.pages.iter().map(|p| (p.col, p.row)).collect();
        let pixels_per_point = painter.ctx().pixels_per_point();
        let snap = |p: Pos2| {
            pos2(
                (p.x * pixels_per_point).round() / pixels_per_point,
                (p.y * pixels_per_point).round() / pixels_per_point,
            )
        };
        let tiles: Vec<_> = note
            .pages
            .iter()
            .enumerate()
            .map(|(pi, page)| {
                let origin = page_origin(
                    page.col,
                    page.row,
                    note.page_w,
                    note.page_h,
                    note.sheet_join.gap(),
                );
                let min = map(Pos2::new(origin.x, origin.y));
                let max = map(Pos2::new(origin.x + note.page_w, origin.y + note.page_h));
                let paper = Rect::from_min_max(min, max);
                let n_left = fused && occupied.contains(&(page.col - 1, page.row));
                let n_right = fused && occupied.contains(&(page.col + 1, page.row));
                let n_top = fused && occupied.contains(&(page.col, page.row - 1));
                let n_bot = fused && occupied.contains(&(page.col, page.row + 1));
                (pi, page, origin, paper, n_left, n_right, n_top, n_bot)
            })
            .collect();

        for &(_, _, _, paper, n_left, n_right, n_top, n_bot) in &tiles {
            if !paper.expand(5.0).intersects(rect) {
                continue;
            }
            let rad = 5;
            let sheet_r = if fused {
                CornerRadius {
                    nw: if n_left || n_top { 0 } else { rad },
                    ne: if n_right || n_top { 0 } else { rad },
                    sw: if n_left || n_bot { 0 } else { rad },
                    se: if n_right || n_bot { 0 } else { rad },
                }
            } else {
                CornerRadius::same(rad)
            };
            let shadow = if fused {
                let mut sh = paper;
                if !n_right {
                    sh.max.x += 3.0;
                }
                if !n_bot {
                    sh.max.y += 4.0;
                }
                sh
            } else {
                paper.translate(vec2(3.0, 4.0))
            };
            painter.rect_filled(shadow, sheet_r, self.look.shadow);
        }

        // Finish every opaque sheet before drawing grain, ruling or user content.
        // A later page must never paint over its neighbour's grid or ink.
        let mut surfaces = Vec::with_capacity(tiles.len());
        for &(_, page, _, paper, n_left, n_right, n_top, n_bot) in &tiles {
            if !paper.expand(bleed).intersects(rect) {
                continue;
            }
            let rad = 5;
            let sheet_r = if fused {
                CornerRadius {
                    nw: if n_left || n_top { 0 } else { rad },
                    ne: if n_right || n_top { 0 } else { rad },
                    sw: if n_left || n_bot { 0 } else { rad },
                    se: if n_right || n_bot { 0 } else { rad },
                }
            } else {
                CornerRadius::same(rad)
            };
            let mut body = paper;
            if fused {
                if n_left {
                    body.min.x -= bleed;
                }
                if n_right {
                    body.max.x += bleed;
                }
                if n_top {
                    body.min.y -= bleed;
                }
                if n_bot {
                    body.max.y += bleed;
                }
            }
            painter.rect_filled(body, sheet_r, fill);
            surfaces.push((page.background.is_some(), paper, body, sheet_r));
        }

        for (has_bg, paper, body, sheet_r) in surfaces {
            // Adjacent cells share exactly the same physical-pixel scissor edge.
            // Extended geometry supplies coverage without blending twice at seams.
            let surface_clip = if fused {
                Rect::from_min_max(snap(paper.min), snap(paper.max))
            } else {
                paper
            };
            let surface_painter = painter.with_clip_rect(surface_clip.intersect(rect));
            if has_bg {
                continue;
            }
            if let Some(tex) = &canson {
                // World-space grain stays attached to the paper while panning and zooming.
                let a = cam.to_paper(body.min, rect);
                let b = cam.to_paper(body.max, rect);
                let uv = Rect::from_min_max(
                    pos2(a.x / PAPER_TILE, a.y / PAPER_TILE),
                    pos2(b.x / PAPER_TILE, b.y / PAPER_TILE),
                );
                surface_painter.add(
                    egui::epaint::RectShape::filled(
                        body,
                        sheet_r,
                        Color32::WHITE.gamma_multiply(0.20),
                    )
                    .with_texture(tex.id(), uv),
                );
            }
            if fused {
                let visible = surface_clip.intersect(rect).expand(2.0 / pixels_per_point);
                self.paint_template_world(&surface_painter, visible, note.paper, cam, rect);
            } else {
                self.paint_template(&surface_painter, paper, note.paper, cam, rect, false);
            }
        }

        for &(pi, page, origin, paper, _, n_right, n_top, _) in &tiles {
            let has_bg = page.background.is_some();
            if !has_bg && note.paper == PaperKind::Lined && page.col == min_col {
                self.paint_punches(painter, paper, cam.zoom);
            }
            if !has_bg
                && note.paper != PaperKind::Slate
                && !n_right
                && !n_top
                && !note.can_tear_unit()
            {
                paint_page_fold(painter, paper, cam.zoom, self.look.paper_rule_strong);
            }
            if has_bg && paper.expand(bleed).intersects(rect) {
                if let Some(bg) = page.background.as_ref() {
                    let [x, y, w, h] = bg.fit(note.page_w, note.page_h);
                    let r = Rect::from_min_size(
                        map(pos2(origin.x + x, origin.y + y)),
                        vec2(w * cam.zoom, h * cam.zoom),
                    );
                    if let Some(tex) =
                        self.background_tex(painter.ctx(), note.id, bg, cam.zoom, pixels_per_point)
                    {
                        painter.image(
                            tex.id(),
                            r,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                }
            }

            for im in &page.images {
                let r = Rect::from_min_size(
                    map(im.min() + origin),
                    vec2(im.size[0] * cam.zoom, im.size[1] * cam.zoom),
                );
                if let Some(tex) = self.texture_for(painter.ctx(), &note.id, &im.file) {
                    painter.image(
                        tex.id(),
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    painter.rect_filled(r, 0.0, self.look.paper_rule);
                }
            }

            if let Some(n) = &mut self.note {
                if let Some(pg) = n.pages.get_mut(pi) {
                    for s in &mut pg.strokes {
                        let mesh = s.tessellate().clone();
                        painter.add(Shape::mesh(map_mesh(&mesh, |p| map(p + origin))));
                    }
                }
            }

            for tx in &page.texts {
                if self.editing_text == Some((pi, tx.id)) {
                    if tx.text.is_empty() {
                        let pos = map(tx.min() + origin);
                        let h = tx.size_pt * cam.zoom;
                        painter.line_segment(
                            [pos + vec2(0.6, 1.0), pos + vec2(0.6, h)],
                            Stroke::new(1.7_f32, tx.color32()),
                        );
                    }
                    continue;
                }
                let pos = map(tx.min() + origin);
                let max_w = (tx.size[0] * cam.zoom).max(8.0);
                let font = FontId::new(tx.size_pt * cam.zoom, FontFamily::Name("serif".into()));
                let galley = painter.layout(tx.text.clone(), font, tx.color32(), max_w);
                painter.galley(pos, galley, tx.color32());
            }

            if !fused && page.background.is_none() {
                painter.text(
                    pos2(paper.center().x, paper.max.y - 14.0 * cam.zoom.min(1.2)),
                    Align2::CENTER_BOTTOM,
                    format!("{}", pi + 1),
                    self.look.serif(12.0),
                    self.look.ink.gamma_multiply(0.38),
                );
            }
        }

        if let Some(pi) = self.live.as_ref().map(|(pi, _)| *pi) {
            let origin = self.origin_of(pi);
            let mesh = if let Some(preview) = self.shape_preview.as_mut() {
                Some(preview.tessellate().clone())
            } else {
                self.live
                    .as_mut()
                    .map(|(_, stroke)| stroke.tessellate().clone())
            };
            if let Some(mesh) = mesh {
                painter.add(Shape::mesh(map_mesh(&mesh, |p| map(p + origin))));
            }
        }
        if self.lasso.len() >= 2 {
            let pts: Vec<_> = self.lasso.iter().copied().map(map).collect();
            painter.add(Shape::closed_line(pts, Stroke::new(1.2, self.look.accent)));
        }
        for s in &self.sel {
            if let Some(n) = &self.note {
                if let Some(page) = n.pages.get(s.page) {
                    let o = self.origin_of(s.page);
                    let bb = match s.kind {
                        SelKind::Stroke => page
                            .strokes
                            .iter()
                            .find(|x| x.id == s.id)
                            .and_then(|st| st.bbox()),
                        SelKind::Text => page.texts.iter().find(|x| x.id == s.id).map(|t| {
                            let r = t.rect();
                            (r.min, r.max)
                        }),
                        SelKind::Image => page.images.iter().find(|x| x.id == s.id).map(|t| {
                            let r = t.rect();
                            (r.min, r.max)
                        }),
                    };
                    if let Some((a, b)) = bb {
                        draw_ants(painter, map(a + o), map(b + o), self.look.accent);
                    }
                }
            }
        }
    }

    fn unit_screen_rect(&self, canvas: Rect, col: i32, row: i32) -> Rect {
        let o = page_origin(col, row, PAGE_W, PAGE_H, self.gap());
        let min = self.camera.to_screen(Pos2::new(o.x, o.y), canvas);
        let max = self
            .camera
            .to_screen(Pos2::new(o.x + PAGE_W, o.y + PAGE_H), canvas);
        Rect::from_min_max(min, max)
    }

    fn sheet_tabs(&self, canvas: Rect) -> Vec<(i32, i32, Rect)> {
        let Some(n) = &self.note else {
            return Vec::new();
        };
        if canvas.width() < 10.0 {
            return Vec::new();
        }
        n.unit_tabs()
            .into_iter()
            .filter_map(|t| {
                let hit = if t.center {
                    let hole = self.unit_screen_rect(canvas, t.dest_col, t.dest_row);
                    if hole.width() < 8.0 || hole.height() < 8.0 {
                        return None;
                    }
                    center_band(hole)
                } else {
                    let edge = match (t.dcol, t.drow) {
                        (-1, 0) => SheetEdge::Left,
                        (1, 0) => SheetEdge::Right,
                        (0, -1) => SheetEdge::Top,
                        (0, 1) => SheetEdge::Bottom,
                        _ => return None,
                    };
                    let paper = self.unit_screen_rect(canvas, t.src_col, t.src_row);
                    if paper.width() < 8.0 || paper.height() < 8.0 {
                        return None;
                    }
                    outward_band(paper, edge)
                };
                Some((t.dest_col, t.dest_row, hit))
            })
            .collect()
    }

    fn sheet_tab_at(&self, screen: Pos2, canvas: Rect) -> Option<(i32, i32)> {
        if self.note.is_none() || canvas.width() < 10.0 {
            return None;
        }
        self.sheet_tabs(canvas)
            .into_iter()
            .filter(|(_, _, r)| r.contains(screen))
            .min_by(|a, b| {
                let da = a.2.center().distance(screen);
                let db = b.2.center().distance(screen);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(c, r, _)| (c, r))
    }

    fn add_sheet_from_tab(&mut self, col: i32, row: i32) {
        if self.note.as_ref().is_none_or(|n| n.unit_occupied(col, row)) {
            return;
        }
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.push_snapshot();
        if let Some(n) = &mut self.note {
            n.add_unit_at(col, row);
        }
        self.sel.clear();
        self.lasso.clear();
        self.mark_dirty();
    }

    fn paint_sheet_tabs(&self, ui: &Ui, painter: &Painter, canvas: Rect) {
        if self.note.is_none() || canvas.width() < 10.0 {
            return;
        }
        let hover = ui.input(|i| i.pointer.hover_pos());
        let (wash, plus, wash_hot, plus_hot) = sheet_tab_colors(self.look.desk, self.look.green());
        for (_, _, hit) in self.sheet_tabs(canvas) {
            if !hit.is_positive() {
                continue;
            }
            let hot = hover.is_some_and(|p| hit.contains(p));
            paint_sheet_tab(
                painter,
                hit,
                if hot { wash_hot } else { wash },
                if hot { plus_hot } else { plus },
            );
        }
    }

    fn start_page_delete(&mut self) {
        if self.page_delete.is_some() {
            return;
        }
        let Some(cells) = self
            .note
            .as_ref()
            .filter(|n| n.can_tear_unit())
            .map(|n| n.unit_cells())
        else {
            return;
        };
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.sel.clear();
        self.lasso.clear();
        self.page_delete_view = Some((self.camera, self.canvas_rect));
        if Camera::valid_viewport(self.canvas_rect) {
            let mut bounds = Rect::NOTHING;
            for (col, row) in cells {
                let origin = page_origin(col, row, PAGE_W, PAGE_H, self.gap());
                bounds = bounds.union(Rect::from_min_size(origin.to_pos2(), vec2(PAGE_W, PAGE_H)));
            }
            self.camera.fit_page(
                self.canvas_rect,
                bounds.min.to_vec2(),
                bounds.width(),
                bounds.height(),
            );
            self.fitted_cell = None;
            self.need_fit = false;
        }
        self.page_delete = Some(Vec::new());
    }

    fn cancel_page_delete(&mut self) {
        self.page_delete = None;
        if let Some((mut camera, viewport)) = self.page_delete_view.take() {
            camera.resize_viewport(viewport, self.canvas_rect, None);
            self.camera = camera;
        }
    }

    fn delete_selected_pages(&mut self) {
        let Some(selected) = self.page_delete.as_ref() else {
            return;
        };
        let Some(note) = self.note.as_ref() else {
            return;
        };
        let cells = note.unit_cells();
        let selected: Vec<_> = cells
            .iter()
            .copied()
            .filter(|c| selected.contains(c))
            .collect();
        if selected.is_empty() || selected.len() >= cells.len() {
            return;
        }
        self.push_snapshot();
        if let Some(note) = &mut self.note {
            for (col, row) in selected {
                note.remove_unit(col, row);
            }
        }
        self.page_delete = None;
        self.page_delete_view = None;
        self.fitted_cell = None;
        self.need_fit = true;
        self.mark_dirty();
    }

    fn paint_page_selection(&mut self, ui: &Ui, painter: &Painter, canvas: Rect) {
        let cells = self
            .note
            .as_ref()
            .map(|n| n.unit_cells())
            .unwrap_or_default();
        let mut toggle = None;
        for (index, &(col, row)) in cells.iter().enumerate() {
            let paper = self.unit_screen_rect(canvas, col, row);
            let hit = paper.intersect(canvas);
            if !hit.is_positive() {
                continue;
            }
            let selected = self
                .page_delete
                .as_ref()
                .is_some_and(|s| s.contains(&(col, row)));
            let resp = ui.interact(
                hit,
                Id::new("select-delete-page").with((col, row)),
                Sense::click(),
            );
            resp.widget_info(|| {
                WidgetInfo::selected(
                    WidgetType::Checkbox,
                    ui.is_enabled(),
                    selected,
                    format!("Page {}", index + 1),
                )
            });
            let color = if selected {
                self.look.rust()
            } else {
                self.look.accent
            };
            painter.rect_filled(
                paper,
                CornerRadius::ZERO,
                color.gamma_multiply(if selected { 0.16 } else { 0.035 }),
            );
            painter.rect_stroke(
                paper.shrink(2.0),
                CornerRadius::same(2),
                Stroke::new(
                    if selected || resp.has_focus() {
                        3.0
                    } else {
                        1.0
                    },
                    color,
                ),
                StrokeKind::Inside,
            );
            let badge = Rect::from_min_size(hit.min + vec2(8.0, 8.0), vec2(44.0, 44.0));
            let (face, fg) = self.paint_note_surface(
                ui,
                badge,
                &resp,
                if selected { color } else { self.look.paper },
                10,
            );
            if selected {
                paint_check(painter, face.center(), fg);
            } else {
                painter.text(
                    face.center(),
                    Align2::CENTER_CENTER,
                    (index + 1).to_string(),
                    self.look.mono(18.0),
                    fg,
                );
            }
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                toggle = Some((col, row));
            }
        }
        if let (Some(cell), Some(selected)) = (toggle, &mut self.page_delete) {
            if selected.contains(&cell) {
                selected.retain(|c| *c != cell);
            } else if selected.len() + 1 < cells.len() {
                selected.push(cell);
            }
            ui.ctx().request_repaint();
        }
    }
    fn paint_template(
        &self,
        painter: &Painter,
        paper: Rect,
        kind: PaperKind,
        cam: Camera,
        canvas: Rect,
        fused: bool,
    ) {
        let z = cam.zoom;
        if fused {
            self.paint_template_world(painter, paper, kind, cam, canvas);
            return;
        }
        match kind {
            PaperKind::Blank | PaperKind::Slate => {}
            PaperKind::Lined => {
                let mut y = paper.min.y + 88.0 * z;
                while y < paper.max.y - 24.0 * z {
                    painter.line_segment(
                        [
                            pos2(paper.min.x + 56.0 * z, y),
                            pos2(paper.max.x - 24.0 * z, y),
                        ],
                        Stroke::new(1.0, self.look.paper_rule),
                    );
                    y += 28.0 * z;
                }
                painter.line_segment(
                    [
                        pos2(paper.min.x + 64.0 * z, paper.min.y + 24.0 * z),
                        pos2(paper.min.x + 64.0 * z, paper.max.y - 24.0 * z),
                    ],
                    Stroke::new(1.15, {
                        let red = self.look.inks.get(2).copied().unwrap_or(self.look.ink);
                        Color32::from_rgb(
                            (self.look.paper.r() as f32 * 0.62 + red.r() as f32 * 0.38) as u8,
                            (self.look.paper.g() as f32 * 0.62 + red.g() as f32 * 0.38) as u8,
                            (self.look.paper.b() as f32 * 0.62 + red.b() as f32 * 0.38) as u8,
                        )
                    }),
                );
            }
            PaperKind::Grid => {
                let mut x = paper.min.x + 24.0 * z;
                while x < paper.max.x {
                    painter.line_segment(
                        [
                            pos2(x, paper.min.y + 24.0 * z),
                            pos2(x, paper.max.y - 24.0 * z),
                        ],
                        Stroke::new(0.8, self.look.paper_rule),
                    );
                    x += 24.0 * z;
                }
                let mut y = paper.min.y + 24.0 * z;
                while y < paper.max.y {
                    painter.line_segment(
                        [
                            pos2(paper.min.x + 24.0 * z, y),
                            pos2(paper.max.x - 24.0 * z, y),
                        ],
                        Stroke::new(0.8, self.look.paper_rule),
                    );
                    y += 24.0 * z;
                }
            }
            PaperKind::Dots => {
                let mut y = paper.min.y + 32.0 * z;
                while y < paper.max.y - 16.0 * z {
                    let mut x = paper.min.x + 32.0 * z;
                    while x < paper.max.x - 16.0 * z {
                        painter.circle_filled(
                            pos2(x, y),
                            1.1 * z.max(0.6),
                            self.look.paper_rule_strong,
                        );
                        x += 22.0 * z;
                    }
                    y += 22.0 * z;
                }
            }
            PaperKind::Millimetre => {
                let mut i = 0;
                let mut y = paper.min.y + 40.0 * z;
                while y < paper.max.y - 20.0 * z {
                    let c = if i % 5 == 0 {
                        self.look.paper_rule_strong
                    } else {
                        self.look.paper_rule
                    };
                    painter.line_segment(
                        [
                            pos2(paper.min.x + 48.0 * z, y),
                            pos2(paper.max.x - 20.0 * z, y),
                        ],
                        Stroke::new(if i % 5 == 0 { 1.0 } else { 0.6 }, c),
                    );
                    y += 8.0 * z;
                    i += 1;
                }
                let mut i = 0;
                let mut x = paper.min.x + 48.0 * z;
                while x < paper.max.x - 20.0 * z {
                    let c = if i % 5 == 0 {
                        self.look.paper_rule_strong
                    } else {
                        self.look.paper_rule
                    };
                    painter.line_segment(
                        [
                            pos2(x, paper.min.y + 40.0 * z),
                            pos2(x, paper.max.y - 20.0 * z),
                        ],
                        Stroke::new(if i % 5 == 0 { 1.0 } else { 0.6 }, c),
                    );
                    x += 8.0 * z;
                    i += 1;
                }
            }
        }
    }

    fn paint_template_world(
        &self,
        painter: &Painter,
        clip: Rect,
        kind: PaperKind,
        cam: Camera,
        canvas: Rect,
    ) {
        let z = cam.zoom;
        let map = |p: Pos2| cam.to_screen(p, canvas);
        let a = cam.to_paper(clip.min, canvas);
        let b = cam.to_paper(clip.max, canvas);
        let xmin = a.x.min(b.x);
        let xmax = a.x.max(b.x);
        let ymin = a.y.min(b.y);
        let ymax = a.y.max(b.y);
        match kind {
            PaperKind::Blank | PaperKind::Slate => {}
            PaperKind::Lined => {
                let mut y = 88.0 + ((ymin - 88.0) / 28.0).floor() * 28.0;
                while y < ymax {
                    if y > ymin {
                        let s = map(Pos2::new(xmin, y));
                        hline(
                            painter,
                            s.y,
                            clip.min.x,
                            clip.max.x,
                            clip,
                            Stroke::new(1.0, self.look.paper_rule),
                        );
                    }
                    y += 28.0;
                }
                let red = self.look.inks.get(2).copied().unwrap_or(self.look.ink);
                let margin = Color32::from_rgb(
                    (self.look.paper.r() as f32 * 0.62 + red.r() as f32 * 0.38) as u8,
                    (self.look.paper.g() as f32 * 0.62 + red.g() as f32 * 0.38) as u8,
                    (self.look.paper.b() as f32 * 0.62 + red.b() as f32 * 0.38) as u8,
                );
                let x = map(Pos2::new(64.0, 0.0)).x;
                vline(
                    painter,
                    x,
                    clip.min.y,
                    clip.max.y,
                    clip,
                    Stroke::new(1.15, margin),
                );
            }
            PaperKind::Grid => {
                let mut x = (xmin / 24.0).floor() * 24.0;
                while x <= xmax {
                    let s = map(Pos2::new(x, 0.0)).x;
                    vline(
                        painter,
                        s,
                        clip.min.y,
                        clip.max.y,
                        clip,
                        Stroke::new(0.8, self.look.paper_rule),
                    );
                    x += 24.0;
                }
                let mut y = (ymin / 24.0).floor() * 24.0;
                while y <= ymax {
                    let s = map(Pos2::new(0.0, y)).y;
                    hline(
                        painter,
                        s,
                        clip.min.x,
                        clip.max.x,
                        clip,
                        Stroke::new(0.8, self.look.paper_rule),
                    );
                    y += 24.0;
                }
            }
            PaperKind::Dots => {
                let mut y = (ymin / 22.0).floor() * 22.0;
                while y <= ymax {
                    let mut x = (xmin / 22.0).floor() * 22.0;
                    while x <= xmax {
                        let p = map(Pos2::new(x, y));
                        if clip.expand(2.0).contains(p) {
                            painter.circle_filled(p, 1.1 * z.max(0.6), self.look.paper_rule_strong);
                        }
                        x += 22.0;
                    }
                    y += 22.0;
                }
            }
            PaperKind::Millimetre => {
                let mut y = (ymin / 8.0).floor() * 8.0;
                let mut i = (y / 8.0).round() as i32;
                while y <= ymax {
                    let c = if i.rem_euclid(5) == 0 {
                        self.look.paper_rule_strong
                    } else {
                        self.look.paper_rule
                    };
                    let s = map(Pos2::new(0.0, y)).y;
                    hline(
                        painter,
                        s,
                        clip.min.x,
                        clip.max.x,
                        clip,
                        Stroke::new(if i.rem_euclid(5) == 0 { 1.0 } else { 0.6 }, c),
                    );
                    y += 8.0;
                    i += 1;
                }
                let mut x = (xmin / 8.0).floor() * 8.0;
                let mut i = (x / 8.0).round() as i32;
                while x <= xmax {
                    let c = if i.rem_euclid(5) == 0 {
                        self.look.paper_rule_strong
                    } else {
                        self.look.paper_rule
                    };
                    let s = map(Pos2::new(x, 0.0)).x;
                    vline(
                        painter,
                        s,
                        clip.min.y,
                        clip.max.y,
                        clip,
                        Stroke::new(if i.rem_euclid(5) == 0 { 1.0 } else { 0.6 }, c),
                    );
                    x += 8.0;
                    i += 1;
                }
            }
        }
    }

    fn paint_punches(&self, painter: &Painter, paper: Rect, zoom: f32) {
        let z = zoom.clamp(0.35, 3.0);
        for k in 0..3 {
            let y = paper.min.y + paper.height() * (0.22 + k as f32 * 0.28);
            let c = pos2(paper.min.x + 22.0 * z, y);
            painter.circle_filled(c, 7.0 * z, self.look.punch);
            painter.circle_stroke(c, 7.0 * z, Stroke::new(1.0, self.look.paper_rule_strong));
            painter.circle_filled(c, 3.2 * z, self.look.desk);
        }
    }

    fn paint_overlays(&mut self, ui: &mut Ui, rect: Rect) {
        let mut edited = false;
        let want_focus = self.text_focus;
        let mut took_focus = false;
        if let Some((pg, id)) = self.editing_text {
            let cam = self.camera;
            if let Some(note) = &mut self.note {
                let (pw, ph) = note.page_size();
                let gap = note.sheet_join.gap();
                if let Some(page) = note.pages.get_mut(pg) {
                    let origin = page_origin(page.col, page.row, pw, ph, gap);
                    if let Some(tx) = page.texts.iter_mut().find(|t| t.id == id) {
                        let min = cam.to_screen(tx.min() + origin, rect);
                        let size = vec2(tx.size[0] * cam.zoom, tx.size[1] * cam.zoom);
                        let size_pt = tx.size_pt;
                        let col = tx.color32();
                        Area::new(Id::new(("tx", id)))
                            .fixed_pos(min)
                            .order(Order::Foreground)
                            .constrain(false)
                            .show(ui.ctx(), |ui| {
                                ui.set_min_size(size);
                                let te = TextEdit::multiline(&mut tx.text)
                                    .id(Id::new(("tx-edit", id)))
                                    .font(FontId::new(
                                        size_pt * cam.zoom,
                                        FontFamily::Name("serif".into()),
                                    ))
                                    .text_color(col)
                                    .desired_width(size.x)
                                    .desired_rows(2)
                                    .lock_focus(true)
                                    .frame(false);
                                let r = ui.add(te);
                                if r.changed() {
                                    edited = true;
                                }
                                if want_focus {
                                    r.request_focus();
                                    took_focus = true;
                                }
                            });
                    }
                }
            }
        }
        if took_focus {
            self.text_focus = false;
        }
        if edited {
            self.dirty = true;
            self.last_change = Instant::now();
        }
    }

    fn cursor_for_tool(&self, ui: &Ui, resp: &Response) {
        if self.cursor_off {
            ui.ctx().set_cursor_icon(CursorIcon::None);
            return;
        }
        if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
            if self.sheet_tab_at(p, self.canvas_rect).is_some() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                return;
            }
        }
        if !resp.hovered() {
            return;
        }
        let space = ui.input(|i| i.key_down(Key::Space) || i.pointer.middle_down());
        if space {
            ui.ctx()
                .set_cursor_icon(if ui.input(|i| i.pointer.any_down()) {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                });
            return;
        }
        if self.is_tablette() && self.finger_alive(ui) {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
            return;
        }
        let icon = match self.tool {
            Tool::Text => CursorIcon::Text,
            Tool::Lasso => CursorIcon::Crosshair,
            Tool::Image => CursorIcon::Copy,
            Tool::EraserStroke | Tool::EraserArea => CursorIcon::NotAllowed,
            _ => CursorIcon::Crosshair,
        };
        ui.ctx().set_cursor_icon(icon);
    }

    fn pen_over_chrome(ctx: &Context, pos: Pos2) -> bool {
        ctx.data(|d| {
            d.get_temp::<Rect>(Id::new("dock-rect"))
                .map(|r| r.expand(8.0).contains(pos))
                .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("top-rect"))
                    .map(|r| r.expand(4.0).contains(pos))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("tin-rect"))
                    .map(|r| r.expand(4.0).contains(pos))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("strip-rect"))
                    .map(|r| r.expand(4.0).contains(pos))
                    .unwrap_or(false)
        })
    }

    /// Stylus to egui pointer: hover and clicks on the chrome (pencil case, ruler, shelf).
    fn inject_pen_pointer(&mut self, ctx: &Context, raw: &mut egui::RawInput) {
        let pen = self.tablet.snapshot();
        if pen.in_proximity || pen.down {
            if let Some(pos) = pen.pos {
                raw.events.push(Event::PointerMoved(pos));
                // On the shelf, every click. On the lectern, chrome only,
                // and only when the press began on the chrome, so a stroke
                // drifting onto the pencil case stays ink.
                if pen.pressed {
                    self.pen_ui_grab = match self.scene {
                        Scene::Shelf { .. } => true,
                        Scene::Desk => Self::pen_over_chrome(ctx, pos),
                    };
                }
                if !pen.down && !pen.pressed {
                    self.pen_ui_grab = false;
                }
                let ui_click = match self.scene {
                    Scene::Shelf { .. } => true,
                    Scene::Desk => self.pen_ui_grab,
                };
                if ui_click {
                    let mods = raw.modifiers;
                    if pen.pressed {
                        raw.events.push(Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers: mods,
                        });
                    }
                    if pen.released {
                        raw.events.push(Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: false,
                            modifiers: mods,
                        });
                        self.pen_ui_grab = false;
                    }
                } else if pen.released {
                    self.pen_ui_grab = false;
                }
            }
            self.pen_was_prox = true;
        } else if self.pen_was_prox {
            raw.events.push(Event::PointerGone);
            self.pen_was_prox = false;
            self.pen_ui_grab = false;
        }
    }

    fn sync_pen_cursor(&mut self, ctx: &Context) {
        let pen = self.tablet.snapshot();
        let hide = pen.in_proximity || pen.down;
        if hide {
            ctx.set_cursor_icon(CursorIcon::None);
            if !self.cursor_off {
                ctx.send_viewport_cmd(ViewportCommand::CursorVisible(false));
                self.cursor_off = true;
            }
        } else if self.cursor_off {
            ctx.send_viewport_cmd(ViewportCommand::CursorVisible(true));
            self.cursor_off = false;
        }
    }

    fn background_tex(
        &mut self,
        ctx: &Context,
        note_id: Uuid,
        bg: &PageBackground,
        zoom: f32,
        pixels_per_point: f32,
    ) -> Option<TextureHandle> {
        let scale = if bg.pdf_page.is_some() {
            crate::pdf::render_scale(zoom, pixels_per_point)
        } else {
            1.0
        };
        let key = format!(
            "{note_id}:{}:{}:{scale:.1}",
            bg.file,
            bg.pdf_page.unwrap_or(u32::MAX)
        );
        if self.bg_textures.contains_key(&key) {
            if let Some(pos) = self.bg_order.iter().position(|(k, _)| k == &key) {
                if let Some(entry) = self.bg_order.remove(pos) {
                    self.bg_order.push_back(entry);
                }
            }
            return self.bg_textures.get(&key).cloned();
        }
        if self.bg_failed.contains(&key) {
            return None;
        }
        let path = self.lib.note_dir(note_id).join(&bg.file);
        let rendered = if let Some(page) = bg.pdf_page {
            crate::pdf::render_pdf_file(&path, page, scale).ok()
        } else {
            image::open(&path).ok().and_then(|img| {
                let img = img.into_rgba8();
                let width = img.width();
                let height = img.height();
                if width == 0 || height == 0 {
                    None
                } else {
                    Some(crate::pdf::Rendered {
                        width,
                        height,
                        rgba: img.into_raw(),
                    })
                }
            })
        };
        let Some(rendered) = rendered else {
            self.bg_failed.insert(key);
            return None;
        };
        if rendered.width == 0 || rendered.height == 0 || rendered.rgba.is_empty() {
            self.bg_failed.insert(key);
            return None;
        }
        let bytes = rendered.rgba.len();
        let image = ColorImage::from_rgba_unmultiplied(
            [rendered.width as usize, rendered.height as usize],
            &rendered.rgba,
        );
        let tex = ctx.load_texture(key.clone(), image, TextureOptions::LINEAR);
        self.bg_bytes += bytes;
        self.bg_order.push_back((key.clone(), bytes));
        self.bg_textures.insert(key, tex.clone());
        const BUDGET: usize = 96 * 1024 * 1024;
        while self.bg_bytes > BUDGET {
            let Some((old, old_bytes)) = self.bg_order.pop_front() else {
                break;
            };
            if self.bg_textures.remove(&old).is_some() {
                self.bg_bytes = self.bg_bytes.saturating_sub(old_bytes);
            }
        }
        Some(tex)
    }

    fn texture_for(&mut self, ctx: &Context, note_id: &Uuid, file: &str) -> Option<TextureHandle> {
        if let Some(t) = self.textures.get(file) {
            return Some(t.clone());
        }
        let path = self.lib.media_path(*note_id, file);
        let img = image::open(path).ok()?.into_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let eg = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        let tex = ctx.load_texture(file.to_string(), eg, TextureOptions::LINEAR);
        self.textures.insert(file.to_string(), tex.clone());
        Some(tex)
    }

    fn export_png(&mut self, ctx: &Context) {
        let Some(note) = self.note.clone() else {
            return;
        };
        let media = MediaLoader {
            root: self
                .lib
                .media_path(note.id, "")
                .parent()
                .unwrap_or(&self.lib.root)
                .to_path_buf(),
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{}.png", slug(&note.title)))
            .save_file()
        else {
            return;
        };
        let note_dir = self.lib.note_dir(note.id);
        if let Some(pm) = export::raster_page(
            &note,
            0,
            &self.look,
            2.0,
            &media,
            &note_dir,
            export::RasterMode::Full,
        ) {
            if let Some(png) = export::pixmap_png(&pm) {
                if std::fs::write(&path, png).is_ok() {
                    let t = ctx.input(|i| i.time);
                    self.toast("Exported", t);
                }
            }
        }
    }

    fn export_pdf(&mut self, ctx: &Context) {
        let Some(note) = self.note.clone() else {
            return;
        };
        let note_dir = self.lib.note_dir(note.id);
        let media = MediaLoader {
            root: note_dir.join("media"),
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{}.pdf", slug(&note.title)))
            .save_file()
        else {
            return;
        };
        if let Some(pdf) = export::pages_pdf(&note, &self.look, &media, &note_dir) {
            if std::fs::write(&path, pdf).is_ok() {
                let t = ctx.input(|i| i.time);
                self.toast("Exported", t);
            }
        }
    }
}

fn snap_dock(pos: Pos2, screen: Rect) -> DockEdge {
    let dl = (pos.x - screen.left()).abs();
    let dr = (pos.x - screen.right()).abs();
    let dt = (pos.y - screen.top()).abs();
    let db = (pos.y - screen.bottom()).abs();
    let m = dl.min(dr).min(dt).min(db);
    if m == dt {
        DockEdge::Top
    } else if m == db {
        DockEdge::Bottom
    } else if m == dl {
        DockEdge::Left
    } else {
        DockEdge::Right
    }
}

fn slug(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "note".into()
    } else {
        s
    }
}

fn next_width(w: f32) -> f32 {
    const WIDTHS: [f32; 3] = [2.2, 5.0, 11.0];
    let i = WIDTHS
        .iter()
        .position(|&p| (w - p).abs() < 1.6)
        .unwrap_or(0);
    WIDTHS[(i + 1) % WIDTHS.len()]
}

fn same_rgb(a: Color32, b: Color32) -> bool {
    a.r().abs_diff(b.r()) < 8 && a.g().abs_diff(b.g()) < 8 && a.b().abs_diff(b.b()) < 8
}

fn rgb_to_hsv(c: Color32) -> (f32, f32, f32) {
    let r = c.r() as f32 / 255.0;
    let g = c.g() as f32 / 255.0;
    let b = c.b() as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d < 1e-5 {
        0.0
    } else if (max - r).abs() < 1e-5 {
        (60.0 * ((g - b) / d) + 360.0) % 360.0
    } else if (max - g).abs() < 1e-5 {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max < 1e-5 { 0.0 } else { d / max };
    (h / 360.0, s, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color32 {
    let h = ((h % 1.0) + 1.0) % 1.0 * 6.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i as i32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgb(
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

fn paint_sv_field(p: &Painter, rect: Rect, hue: f32) {
    let nx = 18;
    let ny = 12;
    let mut mesh = Mesh::default();
    // Shared vertices interpolate the pigments smoothly, without tiled-cell edges.
    for y in 0..=ny {
        for x in 0..=nx {
            let s = x as f32 / nx as f32;
            let t = y as f32 / ny as f32;
            mesh.colored_vertex(
                rect.min + vec2(s * rect.width(), t * rect.height()),
                hsv_to_rgb(hue, s, 1.0 - t),
            );
        }
    }
    for y in 0..ny {
        for x in 0..nx {
            let a = y * (nx + 1) + x;
            mesh.indices
                .extend([a, a + 1, a + nx + 2, a, a + nx + 2, a + nx + 1]);
        }
    }
    p.add(Shape::mesh(mesh));
}

fn paint_hue_bar(p: &Painter, rect: Rect) {
    let mut mesh = Mesh::default();
    for i in 0..=6 {
        let x = rect.min.x + i as f32 / 6.0 * rect.width();
        let color = hsv_to_rgb(i as f32 / 6.0, 1.0, 1.0);
        mesh.colored_vertex(pos2(x, rect.min.y), color);
        mesh.colored_vertex(pos2(x, rect.max.y), color);
        if i < 6 {
            let a = i * 2;
            mesh.indices.extend([a, a + 2, a + 3, a, a + 3, a + 1]);
        }
    }
    p.add(Shape::mesh(mesh));
}

fn shade_rgb(c: Color32, k: f32) -> Color32 {
    let k = k.clamp(0.12, 1.7);
    Color32::from_rgba_unmultiplied(
        (c.r() as f32 * k).min(255.0) as u8,
        (c.g() as f32 * k).min(255.0) as u8,
        (c.b() as f32 * k).min(255.0) as u8,
        c.a(),
    )
}

fn note_controls_bounds(ctx: &Context) -> Rect {
    let screen = ctx.screen_rect();
    let top = ctx
        .data(|d| d.get_temp::<Rect>(Id::new("top-rect")))
        .map_or(screen.top() + 52.0, |r| r.bottom());
    Rect::from_min_max(
        pos2(screen.left() + 8.0, (top + 8.0).min(screen.bottom() - 1.0)),
        pos2(
            (screen.right() - 8.0).max(screen.left() + 9.0),
            (screen.bottom() - 8.0).max(top + 9.0),
        ),
    )
}

/// Pack a floating accessory into the free space around the existing controls.
fn accessory_rect(
    bounds: Rect,
    blockers: &[Rect],
    wanted: Vec2,
    minimum: Vec2,
    edge: DockEdge,
) -> Rect {
    let anchor = blockers.last().copied().unwrap_or(bounds);
    let preferred = match edge {
        DockEdge::Left => pos2(anchor.right() + 8.0, anchor.center().y - wanted.y * 0.5),
        DockEdge::Right => pos2(
            anchor.left() - 8.0 - wanted.x,
            anchor.center().y - wanted.y * 0.5,
        ),
        DockEdge::Top => pos2(anchor.center().x - wanted.x * 0.5, anchor.bottom() + 8.0),
        DockEdge::Bottom => pos2(
            anchor.center().x - wanted.x * 0.5,
            anchor.top() - 8.0 - wanted.y,
        ),
    };
    let mut spaces = vec![bounds];
    for blocker in blockers {
        let cut = blocker.expand(8.0);
        spaces = spaces
            .into_iter()
            .flat_map(|r| {
                if !r.intersects(cut) {
                    return vec![r];
                }
                vec![
                    Rect::from_min_max(r.min, pos2(r.right(), cut.top().min(r.bottom()))),
                    Rect::from_min_max(pos2(r.left(), cut.bottom().max(r.top())), r.max),
                    Rect::from_min_max(r.min, pos2(cut.left().min(r.right()), r.bottom())),
                    Rect::from_min_max(pos2(cut.right().max(r.left()), r.top()), r.max),
                ]
                .into_iter()
                .filter(|r| r.width() >= 44.0 && r.height() >= 44.0)
                .collect()
            })
            .collect();
    }
    spaces
        .into_iter()
        .filter(|space| space.width() >= minimum.x && space.height() >= minimum.y)
        .map(|space| {
            let size = wanted.min(space.size());
            let pos = preferred.clamp(space.min, space.max - size);
            let rect = Rect::from_min_size(pos, size);
            let fit = (size.x / wanted.x).min(size.y / wanted.y);
            let score = fit * 1_000_000.0 - pos.distance(preferred);
            (rect, score)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|r| r.0)
        .unwrap_or_else(|| Rect::from_min_size(bounds.min, wanted.min(bounds.size())))
}

fn well_glyph(well: Color32, paper: Color32, ink: Color32) -> Color32 {
    let lum = 0.299 * well.r() as f32 + 0.587 * well.g() as f32 + 0.114 * well.b() as f32;
    if lum > 155.0 {
        ink
    } else {
        paper
    }
}

fn mix_col(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
        (a.a() as f32 + (b.a() as f32 - a.a() as f32) * t) as u8,
    )
}

const WHEEL_IN: f32 = 56.0;
const WHEEL_OUT: f32 = 152.0;
const WHEEL_HOLD: f64 = 0.35;
const COLOR_HOLD: f64 = 0.35;
const PAPER_TILE: f32 = 168.0;

fn fitted_wheel(screen: Rect, origin: Pos2) -> (Pos2, f32) {
    let radius = ((screen.width().min(screen.height()) - 32.0) * 0.5).clamp(1.0, WHEEL_OUT);
    let inset = radius + 14.0;
    let bounds = screen.shrink(inset);
    (
        origin.clamp(bounds.min, bounds.max.max(bounds.min)),
        radius / WHEEL_OUT,
    )
}

fn wheel_hit(origin: Pos2, pos: Pos2, colors: bool, n_cloth: usize) -> Option<WheelPick> {
    let v = pos - origin;
    let r = v.length();
    if !(WHEEL_IN..=WHEEL_OUT + 48.0).contains(&r) {
        return None;
    }
    let a = (v.angle() + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU);
    if colors {
        let n = n_cloth.max(1);
        let i = ((a / std::f32::consts::TAU) * n as f32).floor() as usize % n;
        return Some(WheelPick::Cloth(i as u8));
    }
    let i = ((a / std::f32::consts::TAU) * 4.0).floor() as usize % 4;
    Some(
        [
            WheelPick::Pin,
            WheelPick::Color,
            WheelPick::Trash,
            WheelPick::Mark,
        ][i],
    )
}

fn slice_mid(origin: Pos2, i: usize, n: usize, r: f32) -> Pos2 {
    let a =
        -std::f32::consts::FRAC_PI_2 + (i as f32 + 0.5) * std::f32::consts::TAU / n.max(1) as f32;
    origin + Vec2::angled(a) * r
}

fn wedge_span(n: usize, i: usize) -> (f32, f32) {
    let n = n.max(1) as f32;
    let span = std::f32::consts::TAU / n;
    let pad = (3.2 / WHEEL_OUT).min(span * 0.12);
    let a0 = -std::f32::consts::FRAC_PI_2 + i as f32 * span + pad;
    let a1 = -std::f32::consts::FRAC_PI_2 + (i as f32 + 1.0) * span - pad;
    (a0, a1)
}

fn wedge_mesh(
    origin: Pos2,
    r0: f32,
    r1: f32,
    n: usize,
    i: usize,
    color: Color32,
    paper: Option<(TextureId, f32)>,
) -> Shape {
    let (a0, a1) = wedge_span(n, i);
    let steps = 12;
    let mut mesh = Mesh::default();
    if let Some((id, _)) = paper {
        mesh.texture_id = id;
    }
    let vert = |p: Pos2| {
        let uv = if let Some((_, tile)) = paper {
            pos2(p.x / tile, p.y / tile)
        } else {
            Pos2::ZERO
        };
        Vertex { pos: p, uv, color }
    };
    for s in 0..steps {
        let t0 = s as f32 / steps as f32;
        let t1 = (s + 1) as f32 / steps as f32;
        let u0 = a0 + (a1 - a0) * t0;
        let u1 = a0 + (a1 - a0) * t1;
        let i0 = mesh.vertices.len() as u32;
        mesh.vertices.extend([
            vert(origin + Vec2::angled(u0) * r0),
            vert(origin + Vec2::angled(u0) * r1),
            vert(origin + Vec2::angled(u1) * r1),
            vert(origin + Vec2::angled(u1) * r0),
        ]);
        mesh.indices
            .extend([i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]);
    }
    Shape::mesh(mesh)
}

fn wedge_stroke(origin: Pos2, r0: f32, r1: f32, n: usize, i: usize, color: Color32) -> Shape {
    let (a0, a1) = wedge_span(n, i);
    let steps = 12;
    let mut pts = Vec::with_capacity(steps * 2 + 2);
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let a = a0 + (a1 - a0) * t;
        pts.push(origin + Vec2::angled(a) * r1);
    }
    for s in (0..=steps).rev() {
        let t = s as f32 / steps as f32;
        let a = a0 + (a1 - a0) * t;
        pts.push(origin + Vec2::angled(a) * r0);
    }
    Shape::closed_line(pts, Stroke::new(2.0_f32, color))
}

/// 50% of the edge, centered. A bit shorter in thickness, set off from the sheet.
fn outward_band(paper: Rect, edge: SheetEdge) -> Rect {
    let gap = paper.height() * 0.10;
    let thick = paper.height() * 0.22;
    let along_h = paper.height() * 0.50;
    let along_w = paper.width() * 0.50;
    let cy = paper.center().y;
    let cx = paper.center().x;
    match edge {
        SheetEdge::Left => Rect::from_min_max(
            pos2(paper.min.x - gap - thick, cy - along_h * 0.5),
            pos2(paper.min.x - gap, cy + along_h * 0.5),
        ),
        SheetEdge::Right => Rect::from_min_max(
            pos2(paper.max.x + gap, cy - along_h * 0.5),
            pos2(paper.max.x + gap + thick, cy + along_h * 0.5),
        ),
        SheetEdge::Top => Rect::from_min_max(
            pos2(cx - along_w * 0.5, paper.min.y - gap - thick),
            pos2(cx + along_w * 0.5, paper.min.y - gap),
        ),
        SheetEdge::Bottom => Rect::from_min_max(
            pos2(cx - along_w * 0.5, paper.max.y + gap),
            pos2(cx + along_w * 0.5, paper.max.y + gap + thick),
        ),
    }
}

fn center_band(paper: Rect) -> Rect {
    let w = paper.width() * 0.42;
    let h = paper.height() * 0.28;
    Rect::from_center_size(paper.center(), vec2(w, h))
}

fn sheet_tab_colors(desk: Color32, green: Color32) -> (Color32, Color32, Color32, Color32) {
    let lum = desk.r() as u16 + desk.g() as u16 + desk.b() as u16;
    if lum < 420 {
        (
            green.gamma_multiply(0.16),
            mix_col(green, Color32::WHITE, 0.20),
            green.gamma_multiply(0.26),
            mix_col(green, Color32::WHITE, 0.36),
        )
    } else {
        (
            green.gamma_multiply(0.14),
            shade_rgb(green, 0.55),
            green.gamma_multiply(0.24),
            shade_rgb(green, 0.42),
        )
    }
}

fn hline(p: &Painter, y: f32, x0: f32, x1: f32, clip: Rect, stroke: Stroke) {
    if y < clip.min.y - 0.6 || y > clip.max.y + 0.6 {
        return;
    }
    let lo = x0.max(clip.min.x);
    let hi = x1.min(clip.max.x);
    if hi > lo + 0.2 {
        p.line_segment([pos2(lo, y), pos2(hi, y)], stroke);
    }
}

fn vline(p: &Painter, x: f32, y0: f32, y1: f32, clip: Rect, stroke: Stroke) {
    if x < clip.min.x - 0.6 || x > clip.max.x + 0.6 {
        return;
    }
    let lo = y0.max(clip.min.y);
    let hi = y1.min(clip.max.y);
    if hi > lo + 0.2 {
        p.line_segment([pos2(x, lo), pos2(x, hi)], stroke);
    }
}

fn paint_page_fold(p: &Painter, paper: Rect, zoom: f32, color: Color32) {
    let s = 16.0 * zoom;
    let fold = [
        pos2(paper.max.x, paper.min.y),
        pos2(paper.max.x - s, paper.min.y),
        pos2(paper.max.x, paper.min.y + s),
    ];
    p.add(Shape::convex_polygon(fold.to_vec(), color, Stroke::NONE));
}

fn paint_sheet_tab(p: &Painter, rect: Rect, wash: Color32, plus: Color32) {
    let short = rect.width().min(rect.height());
    let radius = short * 0.10;
    p.rect_filled(rect, radius, wash);
    let c = rect.center();
    let arm = short * 0.18;
    let thick = (short * 0.018).max(1.0);
    p.rect_filled(
        Rect::from_center_size(c, vec2(arm * 2.0, thick)),
        thick * 0.5,
        plus,
    );
    p.rect_filled(
        Rect::from_center_size(c, vec2(thick, arm * 2.0)),
        thick * 0.5,
        plus,
    );
}

fn paint_open_icon(p: &Painter, c: Pos2, fg: Color32) {
    let st = Stroke::new(1.7, fg);
    let page = Rect::from_center_size(c + vec2(-1.0, 1.0), vec2(14.0, 16.0));
    p.rect_stroke(page, 2.0, st, StrokeKind::Inside);
    p.line_segment(
        [
            pos2(page.max.x - 5.0, page.min.y),
            pos2(page.max.x, page.min.y + 5.0),
        ],
        st,
    );
    p.line_segment([pos2(c.x + 2.0, c.y + 6.5), pos2(c.x + 8.5, c.y + 1.0)], st);
    p.line_segment([pos2(c.x + 4.5, c.y + 1.0), pos2(c.x + 8.5, c.y + 1.0)], st);
    p.line_segment([pos2(c.x + 8.5, c.y + 1.0), pos2(c.x + 8.5, c.y + 5.0)], st);
}

fn paint_plus(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.rect_filled(Rect::from_center_size(c, vec2(16.0, 3.0)), 2.0, fg);
    p.rect_filled(Rect::from_center_size(c, vec2(3.0, 16.0)), 2.0, fg);
}

fn paint_more(p: &egui::Painter, c: Pos2, fg: Color32) {
    for dy in [-7.4, 0.0, 7.4] {
        p.circle_filled(pos2(c.x, c.y + dy), 2.1, fg);
    }
}

fn paper_grain(rect: Rect, radius: u8, paper: &TextureHandle) -> Shape {
    egui::epaint::RectShape::filled(rect, radius, Color32::WHITE.gamma_multiply(0.20))
        .with_texture(
            paper.id(),
            Rect::from_min_max(
                Pos2::ZERO,
                pos2(rect.width() / PAPER_TILE, rect.height() / PAPER_TILE),
            ),
        )
        .into()
}

fn paint_cross(p: &Painter, c: Pos2, arm: f32, fg: Color32) {
    let st = Stroke::new(1.8, fg);
    p.line_segment([c + vec2(-arm, -arm), c + vec2(arm, arm)], st);
    p.line_segment([c + vec2(arm, -arm), c + vec2(-arm, arm)], st);
}

/// Rounded stationery outline, matching the paper control, with an inset cross.
fn paint_delete_page_icon(p: &Painter, c: Pos2, fg: Color32) {
    p.rect_stroke(
        Rect::from_center_size(c, vec2(16.0, 20.0)),
        4.0,
        Stroke::new(1.8, fg),
        StrokeKind::Inside,
    );
    paint_cross(p, c, 3.0, fg);
}

fn paint_back(p: &Painter, c: Pos2, fg: Color32) {
    // The shaft balances the chevron optically inside the circular face.
    p.rect_filled(
        Rect::from_center_size(c + vec2(0.5, 0.0), vec2(17.0, 1.8)),
        0.9,
        fg,
    );
    p.add(Shape::line(
        vec![
            c + vec2(-1.0, -7.0),
            c + vec2(-8.0, 0.0),
            c + vec2(-1.0, 7.0),
        ],
        Stroke::new(1.8, fg),
    ));
}

fn paint_check(p: &Painter, c: Pos2, fg: Color32) {
    p.add(Shape::line(
        vec![
            c + vec2(-5.0, 0.0),
            c + vec2(-1.5, 4.0),
            c + vec2(6.0, -4.5),
        ],
        Stroke::new(1.8, fg),
    ));
}

fn paint_pin_icon(p: &Painter, c: Pos2, fg: Color32) {
    let st = Stroke::new(1.8, fg);
    p.add(Shape::closed_line(
        vec![
            c + vec2(-6.0, -10.0),
            c + vec2(6.0, -10.0),
            c + vec2(4.0, -2.0),
            c + vec2(8.0, 4.0),
            c + vec2(-8.0, 4.0),
            c + vec2(-4.0, -2.0),
        ],
        st,
    ));
    p.line_segment([c + vec2(0.0, 4.0), c + vec2(0.0, 12.0)], st);
}

fn paint_smile_icon(p: &Painter, c: Pos2, fg: Color32) {
    let st = Stroke::new(1.8, fg);
    p.circle_stroke(c, 11.0, st);
    for x in [-3.8, 3.8] {
        p.circle_filled(c + vec2(x, -3.0), 1.3, fg);
    }
    let smile = (0..=12)
        .map(|i| {
            let a = 0.25 + i as f32 / 12.0 * (std::f32::consts::PI - 0.5);
            c + vec2(a.cos() * 5.5, a.sin() * 5.5)
        })
        .collect();
    p.add(Shape::line(smile, st));
}

fn paint_fit(p: &egui::Painter, c: Pos2, fg: Color32) {
    let st = Stroke::new(1.8_f32, fg);
    let s = 7.6;
    let m = 2.6;
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        p.add(Shape::line(
            vec![
                c + vec2(x * s, y * m),
                c + vec2(x * s, y * s),
                c + vec2(x * m, y * s),
            ],
            st,
        ));
    }
}

fn paint_curved_arrow(p: &egui::Painter, c: Pos2, fg: Color32, flip: f32) {
    let mut pts = Vec::with_capacity(15);
    let start = -2.55_f32;
    let end = 0.42_f32;
    let n = 14;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let a = start + (end - start) * t;
        pts.push(pos2(c.x + flip * a.cos() * 8.1, c.y + a.sin() * 8.1 + 1.1));
    }
    p.add(egui::Shape::line(pts.clone(), Stroke::new(1.8_f32, fg)));
    if pts.len() >= 2 {
        let tip = pts[0];
        let nxt = pts[1];
        let dir = (tip - nxt).normalized();
        let nrm = vec2(-dir.y, dir.x);
        p.add(egui::Shape::convex_polygon(
            vec![
                tip + dir * 1.1,
                tip - dir * 5.4 + nrm * 4.0,
                tip - dir * 5.4 - nrm * 4.0,
            ],
            fg,
            Stroke::NONE,
        ));
    }
}

fn paint_undo(p: &egui::Painter, c: Pos2, fg: Color32) {
    paint_curved_arrow(p, c, fg, 1.0);
}

fn paint_redo(p: &egui::Painter, c: Pos2, fg: Color32) {
    paint_curved_arrow(p, c, fg, -1.0);
}

fn paint_text_icon(p: &egui::Painter, c: Pos2, fg: Color32) {
    let wire = Stroke::new(1.55_f32, fg);
    let o = c + vec2(0.0, 0.4);
    let y = o.y - 6.3;
    p.line_segment([pos2(o.x - 7.1, y), pos2(o.x + 7.1, y)], wire);
    p.line_segment([pos2(o.x - 7.1, y), pos2(o.x - 7.1, y + 2.5)], wire);
    p.line_segment([pos2(o.x + 7.1, y), pos2(o.x + 7.1, y + 2.5)], wire);
    p.line_segment([pos2(o.x, y), pos2(o.x, o.y + 6.5)], wire);
    p.line_segment(
        [pos2(o.x - 3.3, o.y + 6.5), pos2(o.x + 3.3, o.y + 6.5)],
        wire,
    );
}

fn paint_image_icon(p: &egui::Painter, c: Pos2, fg: Color32) {
    let wire = Stroke::new(1.55_f32, fg);
    let hair = Stroke::new(1.35_f32, fg);
    let fr = Rect::from_center_size(c + vec2(0.1, 0.15), vec2(15.0, 12.0));
    p.rect_stroke(fr, 2.4, wire, StrokeKind::Inside);
    let x1 = fr.right() - 1.9;
    let y0 = fr.top() + 1.7;
    p.line_segment([pos2(x1 - 3.0, y0), pos2(x1, y0 + 3.0)], hair);
    p.circle_stroke(pos2(fr.left() + 4.3, fr.top() + 4.05), 1.45, hair);
    let base = fr.bottom() - 2.7;
    p.line_segment(
        [pos2(fr.left() + 2.5, base), pos2(c.x - 1.5, c.y - 0.2)],
        hair,
    );
    p.line_segment([pos2(c.x - 1.5, c.y - 0.2), pos2(c.x + 0.4, base)], hair);
    p.line_segment([pos2(c.x - 0.5, base), pos2(c.x + 2.7, c.y + 1.45)], hair);
    p.line_segment(
        [pos2(c.x + 2.7, c.y + 1.45), pos2(fr.right() - 2.3, base)],
        hair,
    );
}

fn paint_paper_icon(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.rect_stroke(
        Rect::from_center_size(c, vec2(14.5, 18.0)),
        4.0,
        Stroke::new(1.8_f32, fg),
        StrokeKind::Inside,
    );
    for dy in [-3.8, 0.0, 3.8] {
        p.line_segment(
            [pos2(c.x - 4.2, c.y + dy), pos2(c.x + 4.2, c.y + dy)],
            Stroke::new(1.55_f32, fg),
        );
    }
}
