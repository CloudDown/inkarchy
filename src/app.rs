#![allow(unknown_lints, float_literal_f32_fallback)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::*;
use egui::containers::scroll_area::{ScrollBarVisibility, ScrollSource};
use egui::epaint::Vertex;
use egui::style::ScrollAnimation;
use uuid::Uuid;

use crate::camera::{page_at, page_origin, Camera, ZOOM_STOPS};
use crate::document::{
    col_runs_on_row, row_runs_on_col, rule_row, snap_rule_y, ImageObj, Note, PageBackground,
    PaperKind, PaperTone, SheetJoin, TextBox, CORNELL_CUE, CORNELL_SUMMARY, PAGE_H, PAGE_W,
};
use crate::emoji::Atlas;
use crate::export::{self, MediaLoader};
use crate::ink::{
    default_width, erase_area, map_mesh, maybe_snap_shape, mixed_pressure, InkPoint, InkStroke,
    Nib, Tool,
};
use crate::library::{
    ensure_png, image_size, DockEdge, Library, SaveError, SHELF_COLS, SHELF_SLOTS, TRASH_SLOTS,
};
use crate::look::Look;
use crate::pressure::Pressure;
use crate::seed;
use crate::tablet::{PenSnapshot, TabletBridge};
use crate::undo::UndoStack;

mod coach;
mod gestures;
#[cfg(test)]
mod tests;
use gestures::TouchGesture;

const DOS_W: f32 = 128.0;
const DOS_H: f32 = 156.0;
const DOS_PAD: f32 = 10.0;
const PAPER_PEEK: f32 = 18.0;
const TITLE_ROW: f32 = 44.0;
/// Extra line under the spine title when Search shows an inner hit.
const SNIPPET_ROW: f32 = 18.0;
const SHELF_GAP: f32 = 8.0;
const TOSS_DURATION: f64 = 0.30;
const TOSS_STAGGER: f64 = 0.035;
const BIN_PANEL_SPEED: f32 = 40.0;
const BIN_REVEAL_DURATION: f32 = 0.13;
const BIN_HOVER_DURATION: f32 = 0.09;
const BIN_DROP_RADIUS: f32 = 176.0;
/// Critically damped motion, integrated analytically so frame rate cannot add bounce.
const DOCK_SPRING_OMEGA: f32 = 28.0;
const DOCK_FOLD_OMEGA: f32 = 32.0;
const DOCK_VEL_MAX: f32 = 2200.0;
/// Pointer distance over which the case eases onto a side while it is carried.
const DOCK_MAGNET: f32 = 140.0;
/// Magnet amount at which a carried case turns to match that side.
const DOCK_TURN: f32 = 0.35;
/// Room left before a long side meets a wall. The case turns while this gap
/// is still open, so the clamp never piles it into the frame and then rotates.
const DOCK_TURN_AIR: f32 = 72.0;
const QUARTER_DISK_CENTROID: f32 = 4.0 / (3.0 * std::f32::consts::PI);
/// Watercolor saucer: a thin pigment rim around the wash.
const TIN_RING: f32 = 14.0;
const TIN_RING_GAP: f32 = 4.0;
const TIN_SIDE: f32 = 168.0;
const NIB_MIN: f32 = 0.9;
const NIB_MAX: f32 = 18.0;
const PARKABLE_TOOLS: [Tool; 8] = [
    Tool::Fineliner,
    Tool::Brush,
    Tool::Pencil,
    Tool::Highlighter,
    Tool::EraserStroke,
    Tool::Hand,
    Tool::Text,
    Tool::Image,
];

fn dock_tool_key(tool: Tool) -> Tool {
    if tool.is_eraser() {
        Tool::EraserStroke
    } else {
        tool
    }
}

#[derive(Clone)]
enum Scene {
    Shelf { query: String },
    Desk,
}

struct ShelfHit {
    updated: chrono::DateTime<chrono::Utc>,
    title: String,
    body: String,
    pdf_keys: Vec<String>,
}

fn search_snippet(hay: &str, q: &str) -> Option<String> {
    let i = hay.find(q)?;
    let mut start = i;
    let mut taken = 0;
    for (idx, _) in hay[..i].char_indices().rev() {
        start = idx;
        taken += 1;
        if taken >= 10 {
            break;
        }
    }
    let chars: Vec<char> = hay[start..]
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .take(22)
        .collect();
    let more = hay[start..].chars().count() > chars.len();
    let s = chars.iter().collect::<String>().trim().to_string();
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(&s);
    if more {
        out.push('…');
    }
    Some(out)
}

#[derive(Clone, Copy)]
enum DosAct {
    Open,
    Select,
    Toggle,
    Range,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PageArrangement {
    Column,
    Row,
    TwoColumns,
    ThreeColumns,
    FourColumns,
}

impl PageArrangement {
    const ALL: [Self; 5] = [
        Self::Column,
        Self::Row,
        Self::TwoColumns,
        Self::ThreeColumns,
        Self::FourColumns,
    ];

    fn cell(self, index: usize) -> (i32, i32) {
        let index = index as i32;
        match self {
            Self::Column => (0, index),
            Self::Row => (index, 0),
            Self::TwoColumns => (index % 2, index / 2),
            Self::ThreeColumns => (index % 3, index / 3),
            Self::FourColumns => (index % 4, index / 4),
        }
    }
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

/// A file-picker/import job which must never block egui's event loop.
struct ImageImport {
    note: Uuid,
    cell: (i32, i32),
    local: Pos2,
    replace: Option<Uuid>,
    result: Receiver<ImageImportResult>,
}

enum ImageImportResult {
    Ready(Vec<u8>),
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelKind {
    Stroke,
    Text,
    Image,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
    Pages,
    FingerDraw,
    EraseStroke,
    EraseArea,
}

/// Pencil case sliding into its edge after the finger lifts.
#[derive(Clone, Copy)]
struct DockSlide {
    vel: Vec2,
    /// At least one frame of the carried pose has been painted.
    shown: bool,
    /// The closed case has reached the destination and may unfold.
    arrived: bool,
}

pub struct InkarchyApp {
    coach: coach::CoachUi,
    look: Look,
    lib: Library,
    scene: Scene,
    note: Option<Note>,
    tool: Tool,
    color_i: usize,
    /// Current ink (theme swatch or a free tint).
    ink: Color32,
    color_custom: bool,
    /// Watercolor saucer is open.
    tin_open: bool,
    color_mixer_open: bool,
    /// One-slot nib ribbon is open.
    nib_open: bool,
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
    /// Opposite corner, initial pointer vector, and applied uniform scale.
    selection_resize: Option<(Pos2, Vec2, f32)>,
    canvas_touch: Option<TouchGesture>,
    /// After a pinch, ignore the remaining finger until every contact lifts.
    touch_navigation: bool,
    editing_text: Option<(usize, Uuid)>,
    dirty: bool,
    last_change: Instant,
    textures: HashMap<String, TextureHandle>,
    emoji: Atlas,
    emoji_tex: HashMap<char, TextureHandle>,
    emoji_tex_small: HashMap<char, TextureHandle>,
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
    /// Layout edge while the case is carried or in flight. During a drag it
    /// follows the side the pointer is meeting, so the case turns in the hand.
    dock_drag_edge: Option<DockEdge>,
    dock_float: Option<Pos2>,
    dock_grab: Vec2,
    /// The grip drag is followed from the pointer after this, so a turn can
    /// change the widget id without dropping the gesture.
    dock_held: bool,
    dock_moved: bool,
    /// Pointer velocity while dragging, carried into the release spring.
    dock_vel: Vec2,
    dock_size: Option<Vec2>,
    dock_slide: Option<DockSlide>,
    /// 0 = tools open, 1 = case closed on the six-dot grip.
    dock_fold: f32,
    dock_fold_vel: f32,
    canvas_rect: Rect,
    /// Click held on a sheet ear: do not lay ink.
    tab_held: bool,
    /// This press began on a page `+` or `−`. Pages mode owns it until lift.
    page_press_owned: bool,
    tablet: TabletBridge,
    live_from_pen: bool,
    /// Mouse cursor hidden (stylus in proximity).
    cursor_off: bool,
    /// Stylus was in proximity on the previous frame (for PointerGone).
    pen_was_prox: bool,
    /// Emoji picker open for this note.
    emoji_pick: Option<Uuid>,
    /// Active iPhone-style tab inside the emoji sheet.
    emoji_group: crate::emoji::EmojiGroup,
    /// Folder title currently being edited.
    rename_id: Option<Uuid>,
    rename_buf: String,
    /// Title position, so the editor can sit outside the scroll.
    rename_rect: Option<Rect>,
    /// The text field was just placed: give it the keyboard.
    text_focus: bool,
    /// Stylus click that began on the chrome (not on the sheet).
    pen_ui_grab: bool,
    /// Open the image picker outside the click (system portal).
    pending_image: Option<(usize, Pos2)>,
    /// Open the file dialog outside the click (system portal).
    pending_open: bool,
    /// Paths sent by a second `inkarchy file.pdf` while this window is open.
    inbox: Option<crate::ipc::Inbox>,
    bg_textures: HashMap<String, TextureHandle>,
    bg_tex_scale: HashMap<String, f32>,
    bg_order: VecDeque<(String, usize)>,
    bg_bytes: usize,
    bg_failed: HashSet<String>,
    bg_pending: HashSet<String>,
    /// Keys drawn on the last paint. LRU must not drop these, or the PDF
    /// flashes back to empty paper while it re-rasters.
    bg_hot: HashSet<String>,
    pdf_raster: Option<crate::pdf::RasterHub>,
    pdf_view_scale: [f32; 3],
    pdf_detail_after: f64,
    startup_toast: Option<String>,
    /// Image picker and decoding run away from the drawing/event loop.
    image_import: Option<ImageImport>,
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
    /// Title + typed text + PDF unicode, rebuilt when a notebook is saved.
    shelf_hits: HashMap<Uuid, ShelfHit>,
    pdf_text: HashMap<String, String>,
    pdf_text_pending: HashSet<String>,
    pdf_text_hub: Option<crate::pdf::TextHub>,
    pdf_lines: HashMap<(Uuid, usize), Vec<f32>>,
    /// Page selected by the touch-friendly PDF stepper.
    pdf_page_target: usize,
    /// Direction, press start, next repeat time for a held page button.
    pdf_page_repeat: Option<(i8, f64, f64)>,
    pdf_page_picker_open: bool,
    pdf_search_query: String,
    pdf_search_open: bool,
    pdf_search_match: usize,
    pdf_search_seen_query: String,
    pdf_search_last_count: usize,

    /// Page `+` and center `−` marks, shown while the Pages button is on.
    sheet_tabs_on: bool,
    /// The page outlined in Pages mode.
    pages_jiggle_cell: Option<(i32, i32)>,
    /// Tools temporarily parked from the dock in the ink panel.
    parked_tools: HashSet<Tool>,
    /// Tool currently being dragged to or from the parking area.
    dock_tool_drag: Option<Tool>,
    /// Marks ignore the pointer until it has lifted after Pages mode opened.
    pages_marks_ready: bool,
    /// Press on a `+` or `−` that commits only if the finger lifts in place.
    page_mark: Option<PageMarkPress>,
    /// Long-press on empty paper to enter Pages manage mode.
    pages_manage_hold: Option<PagesManageHold>,
    /// Long-press on a page body in manage mode, before the drag starts.
    page_drag_hold: Option<PageDragHold>,
    /// Dragging a page toward a neighbour slot (or swap).
    page_drag: Option<PageDrag>,
}

/// A page mark under the pointer. `travel` is how far the press has moved.
struct PageMarkPress {
    kind: PageMark,
    origin: Pos2,
    travel: f32,
}

#[derive(Clone, Copy)]
enum PageMark {
    Add(i32, i32),
    Tear(i32, i32),
    TearSticky(usize, Uuid),
    TearImage(usize, Uuid),
}

#[derive(Clone, Copy)]
struct PagesManageHold {
    col: i32,
    row: i32,
    screen: Pos2,
    t0: f64,
}

#[derive(Clone, Copy)]
struct PageDragHold {
    col: i32,
    row: i32,
    screen: Pos2,
    t0: f64,
}

#[derive(Clone, Copy)]
struct PageDrag {
    col: i32,
    row: i32,
    /// Pointer offset from the page screen origin when the drag armed.
    grab: Vec2,
    screen: Pos2,
}

impl InkarchyApp {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        let look = Look::load();
        look.apply(&cc.egui_ctx);
        let mut lib = Library::open();
        seed::seed_if_needed(&mut lib);
        let mut app = Self::with_library(look, lib);
        if files.is_empty() {
            // INKARCHY_OPEN wins. CAHIER_OPEN is a deprecated alias.
            let open = std::env::var("INKARCHY_OPEN").or_else(|_| std::env::var("CAHIER_OPEN"));
            if let Ok(q) = open {
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
        let parked_tools = lib
            .index
            .parked_tools
            .iter()
            .copied()
            .map(dock_tool_key)
            .collect();
        let ink0 = look.inks.first().copied().unwrap_or(look.ink);
        Self {
            coach: coach::CoachUi::default(),
            look,
            lib,
            scene: Scene::Shelf {
                query: String::new(),
            },
            note: None,
            tool: Tool::Fineliner,
            color_i: 0,
            ink: ink0,
            color_custom: false,
            tin_open: false,
            color_mixer_open: false,
            nib_open: false,
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
            selection_resize: None,
            canvas_touch: None,
            touch_navigation: false,
            editing_text: None,
            dirty: false,
            last_change: Instant::now(),
            textures: HashMap::new(),
            emoji: Atlas::load(),
            emoji_tex: HashMap::new(),
            emoji_tex_small: HashMap::new(),
            pressure: Pressure::start(),
            last_ptr: None,
            toast: None,
            need_fit: true,
            fitted_cell: None,
            land_page: None,
            title_buf: String::new(),
            dock_edge,
            dock_drag_edge: None,
            dock_float: None,
            dock_grab: Vec2::ZERO,
            dock_held: false,
            dock_moved: false,
            dock_vel: Vec2::ZERO,
            dock_size: None,
            dock_slide: None,
            dock_fold: 0.0,
            dock_fold_vel: 0.0,
            canvas_rect: Rect::ZERO,
            tab_held: false,
            page_press_owned: false,
            tablet: TabletBridge::new(),
            live_from_pen: false,
            cursor_off: false,
            pen_was_prox: false,
            emoji_pick: None,
            emoji_group: crate::emoji::EmojiGroup::SmileysPeople,
            rename_id: None,
            rename_buf: String::new(),
            rename_rect: None,
            text_focus: false,
            pen_ui_grab: false,
            pending_image: None,
            pending_open: false,
            inbox: None,
            bg_textures: HashMap::new(),
            bg_tex_scale: HashMap::new(),
            bg_order: VecDeque::new(),
            bg_bytes: 0,
            bg_failed: HashSet::new(),
            bg_pending: HashSet::new(),
            bg_hot: HashSet::new(),
            pdf_raster: None,
            pdf_view_scale: [0.0; 3],
            pdf_detail_after: 0.0,
            startup_toast: None,
            image_import: None,
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
            shelf_hits: HashMap::new(),
            pdf_text: HashMap::new(),
            pdf_text_pending: HashSet::new(),
            pdf_text_hub: None,
            pdf_lines: HashMap::new(),
            pdf_page_target: 0,
            pdf_page_repeat: None,
            pdf_page_picker_open: false,
            pdf_search_query: String::new(),
            pdf_search_open: false,
            pdf_search_match: 0,
            pdf_search_seen_query: String::new(),
            pdf_search_last_count: 0,

            sheet_tabs_on: false,
            pages_jiggle_cell: None,
            parked_tools,
            dock_tool_drag: None,
            pages_marks_ready: false,
            page_mark: None,
            pages_manage_hold: None,
            page_drag_hold: None,
            page_drag: None,
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

    fn note_has_pdf(&self) -> bool {
        self.note.as_ref().is_some_and(|note| {
            note.pages.iter().any(|page| {
                page.background
                    .as_ref()
                    .is_some_and(|bg| bg.pdf_page.is_some())
            })
        })
    }

    fn kick_pdf_preview(&mut self) {
        let (key, path, page, size) = {
            let Some(note) = self.note.as_ref() else {
                return;
            };
            let Some(bg) = note.pages.first().and_then(|p| p.background.as_ref()) else {
                return;
            };
            let Some(page) = bg.pdf_page else {
                return;
            };
            let path = self.lib.note_dir(note.id).join(&bg.file);
            let key = format!("{}:{}:{page}", note.id, bg.file);
            (key, path, page, bg.source_size.unwrap_or([PAGE_W, PAGE_H]))
        };
        let scale = crate::pdf::clamp_scale(size[0], size[1], crate::pdf::preview_scale(1.0))
            .min(2047.0 / size[0].max(size[1]).max(1.0));
        if self.bg_pending.contains(&key) || self.bg_failed.contains(&key) {
            return;
        }
        let hub = self
            .pdf_raster
            .get_or_insert_with(crate::pdf::RasterHub::new);
        if hub.request(crate::pdf::RasterJob {
            key: key.clone(),
            path,
            page,
            scale,
            preview: true,
        }) {
            self.bg_pending.insert(key);
        }
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

    fn zoom_level(&self) -> f32 {
        self.camera.zoom / self.fit_zoom_now().max(0.001)
    }

    fn set_zoom_level(&mut self, level: f32) {
        let rect = self.canvas_rect;
        if !Camera::valid_viewport(rect) {
            return;
        }
        let target = (self.fit_zoom_now() * level).max(0.001);
        self.camera.stretch_y = None;
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
        self.coach.session.changed(false, Instant::now());
        self.touch_document();
    }

    fn touch_document(&mut self) {
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
        self.reset_canvas_gesture();
        self.image_import = None;
        self.pending_image = None;
        self.sheet_tabs_on = false;
        self.pages_jiggle_cell = None;
        self.pages_marks_ready = false;
        self.page_mark = None;
        self.clear_page_drag_state();
        self.page_press_owned = false;
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
            self.camera = Camera::default();
            self.need_fit = false;
            self.land_page = Some(0);
            self.fitted_cell = None;
            self.canvas_rect = Rect::ZERO;
            self.textures.clear();
            self.clear_backgrounds();
            self.kick_pdf_preview();
            self.tab_held = false;
            self.dock_held = false;
            self.dock_fold = 0.0;
            self.dock_fold_vel = 0.0;
            self.pdf_lines.clear();
            self.pdf_page_repeat = None;
            self.pdf_page_picker_open = false;
            self.pdf_search_query.clear();
            self.pdf_search_open = false;
            self.pdf_search_match = 0;
            self.pdf_search_seen_query.clear();
            self.pdf_search_last_count = 0;
        }
    }

    fn close_desk(&mut self) {
        self.finish_live();
        self.coach.session.reset(None);
        self.coach.open = false;
        self.finish_text_edit();
        self.reset_canvas_gesture();
        self.image_import = None;
        self.pending_image = None;
        self.sheet_tabs_on = false;
        self.pages_jiggle_cell = None;
        self.pages_marks_ready = false;
        self.page_mark = None;
        self.clear_page_drag_state();
        self.page_press_owned = false;
        // Save exactly once on the way out. Calling autosave first could
        // serialize a large notebook twice before the shelf became visible.
        if let Some(n) = self.note.take() {
            let saved = self.lib.save_note(&n);
            self.report_save(saved);
        }
        self.dirty = false;
        self.scene = Scene::Shelf {
            query: String::new(),
        };
        self.canvas_rect = Rect::ZERO;
        self.fitted_cell = None;
        self.undo.clear();
        self.sel.clear();
        self.pdf_lines.clear();
        self.pdf_page_repeat = None;
        self.pdf_page_picker_open = false;
        self.pdf_search_query.clear();
        self.pdf_search_open = false;
        self.pdf_search_match = 0;
        self.pdf_search_seen_query.clear();
        self.pdf_search_last_count = 0;
        self.dock_held = false;
        self.dock_fold = 0.0;
        self.dock_fold_vel = 0.0;
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
        let (pw, ph) = self
            .note
            .as_ref()
            .map(|n| n.page_size())
            .unwrap_or((PAGE_W, PAGE_H));
        let idx = page_at(old_paper, &cells, pw, ph, old_gap);
        let (col, row) = cells.get(idx).copied().unwrap_or((0, 0));
        let local = old_paper - page_origin(col, row, pw, ph, old_gap);
        if let Some(n) = &mut self.note {
            n.apply_sheet_join(join);
        }
        if rect.width() > 10.0 {
            let (nw, nh) = self
                .note
                .as_ref()
                .map(|n| n.page_size())
                .unwrap_or((PAGE_W, PAGE_H));
            let o = page_origin(col, row, nw, nh, join.gap());
            let new_world = pos2(o.x + local.x, o.y + local.y);
            let z = self.camera.zoom;
            let zy = self.camera.zoom_y();
            self.camera.pan = (focus - rect.min) - vec2(new_world.x * z, new_world.y * zy);
            self.fitted_cell = None;
        }
        self.sel.clear();
        self.lasso.clear();
        self.mark_dirty();
    }

    fn page_arrangement(&self) -> Option<PageArrangement> {
        let note = self.note.as_ref()?;
        PageArrangement::ALL.into_iter().find(|layout| {
            note.pages
                .iter()
                .enumerate()
                .all(|(index, page)| (page.col, page.row) == layout.cell(index))
        })
    }

    fn arrange_pages(&mut self, layout: PageArrangement) {
        if self.note.as_ref().is_none_or(|note| note.pages.len() < 2)
            || self.page_arrangement() == Some(layout)
        {
            return;
        }
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.clear_page_drag_state();
        self.sel.clear();
        self.lasso.clear();
        let rect = self.canvas_rect;
        let current = if Camera::valid_viewport(rect) {
            self.page_in_view(rect)
        } else {
            0
        };
        let old_origin = self.origin_of(current);
        let local = if Camera::valid_viewport(rect) {
            self.camera.to_paper(rect.center(), rect).to_vec2() - old_origin
        } else {
            Vec2::ZERO
        };
        let was_fitted = self.fitted_cell.is_some();
        self.push_snapshot();
        if let Some(note) = &mut self.note {
            for (index, page) in note.pages.iter_mut().enumerate() {
                (page.col, page.row) = layout.cell(index);
            }
        }
        let cell = layout.cell(current);
        self.pages_jiggle_cell = Some(cell);
        if Camera::valid_viewport(rect) {
            let origin = self.origin_of(current);
            if was_fitted {
                let (pw, ph) = self.note.as_ref().unwrap().page_size();
                self.camera.fit_page(rect, origin, pw, ph);
                self.fitted_cell = Some(cell);
            } else {
                let point = origin + local;
                self.camera.pan = (rect.center() - rect.min)
                    - vec2(point.x * self.camera.zoom, point.y * self.camera.zoom_y());
            }
        }
        self.mark_dirty();
    }

    fn clear_backgrounds(&mut self) {
        self.bg_textures.clear();
        self.bg_tex_scale.clear();
        self.bg_order.clear();
        self.bg_bytes = 0;
        self.bg_failed.clear();
        self.bg_pending.clear();
        self.bg_hot.clear();
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

    /// Remembers the eraser kind from the notebook menu. Live eraser follows.
    fn set_eraser_kind(&mut self, kind: Tool) {
        if !kind.is_eraser() {
            return;
        }
        self.last_eraser = kind;
        if self.tool.is_eraser() {
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

    fn line_guides(&mut self, page: usize) -> Vec<f32> {
        let Some(note) = self.note.as_ref() else {
            return Vec::new();
        };
        let id = note.id;
        if let Some(cached) = self.pdf_lines.get(&(id, page)) {
            return cached.clone();
        }
        let Some(pg) = note.pages.get(page) else {
            return Vec::new();
        };
        let Some(bg) = pg.background.as_ref() else {
            return Vec::new();
        };
        if bg.pdf_page.is_none() {
            return Vec::new();
        }
        let path = self.lib.note_dir(id).join(&bg.file);
        let pdf_page = bg.pdf_page.unwrap_or(0);
        let raw = crate::pdf::text_baselines(&path, pdf_page);
        let (pw, ph) = note.page_size();
        let [_, fy, _, fh] = bg.fit(pw, ph);
        let src_h = bg.source_size.map(|s| s[1]).unwrap_or(ph);
        let mapped: Vec<f32> = raw
            .into_iter()
            .map(|y| fy + (y / src_h.max(1.0)) * fh)
            .collect();
        self.pdf_lines.insert((id, page), mapped.clone());
        mapped
    }

    fn finish_text_edit(&mut self) {
        self.finalize_editing_text();
    }

    fn finalize_editing_text(&mut self) {
        let Some((pg, id)) = self.editing_text.take() else {
            self.text_focus = false;
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
            let sticky = self
                .note
                .as_ref()
                .and_then(|n| n.pages.get(pg))
                .and_then(|p| p.texts.iter().find(|t| t.id == id))
                .is_some_and(|t| t.sticky);
            if !sticky {
                if let Some(n) = &mut self.note {
                    if let Some(page) = n.pages.get_mut(pg) {
                        page.texts.retain(|t| t.id != id);
                    }
                }
            }
            self.mark_dirty();
        }
    }
}

impl eframe::App for InkarchyApp {
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
        self.ingest_pdf_rasters(ctx);

        let on_shelf = matches!(self.scene, Scene::Shelf { .. });
        if on_shelf {
            self.bg_hot.clear();
            if let Some(hub) = &self.pdf_raster {
                for key in hub.retain_visible(&self.bg_hot) {
                    self.bg_pending.remove(&key);
                }
            }
            self.ui_shelf(ctx);
        } else {
            self.ui_desk(ctx);
        }
        self.tick_coach(ctx);

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
        // Full rate only while something is moving. A still selection is a static frame.
        // An unsaved note wakes just often enough for the autosave delay.
        let animating = self.live.is_some()
            || !self.lasso.is_empty()
            || self.dock_float.is_some()
            || self.dock_held
            || self.dock_fold > 0.001
            || self.dock_fold_vel.abs() > 0.01
            || wants_pen
            || self.toast.is_some()
            || pen.in_proximity
            || pen.down;
        if animating && focused {
            ctx.request_repaint();
        } else if self.dirty && focused {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
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
            eprintln!("inkarchy: {err}");
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

impl InkarchyApp {
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
        let mut find_pdf = false;

        let typing = ctx.wants_keyboard_input()
            || self.editing_text.is_some()
            || self.rename_id.is_some()
            || self.emoji_pick.is_some();
        // Keep shortcuts out of credentials/model fields, including Backspace and Ctrl+Z.
        if self.coach.open && typing && !ctx.input(|i| i.key_pressed(Key::Escape)) {
            return;
        }
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
            if c && i.key_pressed(Key::F) && self.note_has_pdf() {
                find_pdf = true;
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
            if !c && !typing && self.editing_text.is_none() {
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
                if i.key_pressed(Key::V) {
                    tool = Some(Tool::Hand);
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

        if find_pdf {
            self.pdf_search_open = true;
            ctx.data_mut(|data| data.insert_temp(Id::new("pdf-search-focus-requested"), true));
        }

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
                    } else if self.pdf_search_open {
                        self.pdf_search_open = false;
                        ctx.memory_mut(|memory| {
                            memory.surrender_focus(Id::new("pdf-search-input"))
                        });
                        ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                    } else if self.coach.open {
                        self.coach.open = false;
                    } else if self.export_picker_open {
                        self.export_picker_open = false;
                    } else if self.ink_panel_open() {
                        self.set_ink_panel(false);
                    } else if self.sheet_tabs_on {
                        self.leave_pages_mode();
                    } else if self.editing_text.is_some() {
                        self.finish_text_edit();
                    } else if !self.sel.is_empty() {
                        self.sel.clear();
                        self.drag_last = None;
                        self.selection_resize = None;
                    } else {
                        self.close_desk();
                    }
                }
                Scene::Shelf { .. } => {
                    // Shelf navigation is handled once in shelf_keys; modals take priority.
                    if self.emoji_pick.is_some() {
                        self.emoji_pick = None;
                        ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                    }
                }
            }
        }
        if let Scene::Desk = self.scene {
            if remove_page {
                self.toggle_pages_mode();
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
                        SelKind::Text => {
                            let sticky = page
                                .texts
                                .iter()
                                .find(|x| x.id == s.id)
                                .is_some_and(|t| t.sticky);
                            page.texts.retain(|x| x.id != s.id);
                            if sticky {
                                page.strokes.retain(|x| x.sticky != Some(s.id));
                            }
                        }
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
                            let old_id = tx.id;
                            let was_sticky = tx.sticky;
                            tx.id = Uuid::new_v4();
                            tx.translate(vec2(16.0, 16.0));
                            neu.push(Sel {
                                page: s.page,
                                id: tx.id,
                                kind: SelKind::Text,
                            });
                            if was_sticky {
                                let copies: Vec<_> = page
                                    .strokes
                                    .iter()
                                    .filter(|st| st.sticky == Some(old_id))
                                    .cloned()
                                    .collect();
                                for mut st in copies {
                                    st.id = Uuid::new_v4();
                                    st.sticky = Some(tx.id);
                                    st.translate(vec2(16.0, 16.0));
                                    page.strokes.push(st);
                                }
                            }
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

    fn ingest_pdf_text(&mut self, ctx: &Context) {
        let Some(hub) = self.pdf_text_hub.as_ref() else {
            return;
        };
        let ready = hub.take_ready();
        if ready.is_empty() {
            return;
        }
        for (key, text) in ready {
            self.pdf_text_pending.remove(&key);
            self.pdf_text.insert(key, text.to_lowercase());
        }
        ctx.request_repaint();
    }

    fn refresh_shelf_search(&mut self) {
        let live: HashSet<Uuid> = self.lib.index.notes.iter().map(|m| m.id).collect();
        self.shelf_hits.retain(|id, _| live.contains(id));
        let metas: Vec<crate::library::NoteMeta> = self.lib.index.notes.clone();
        for meta in metas {
            let stale = self
                .shelf_hits
                .get(&meta.id)
                .map(|hit| hit.updated != meta.updated)
                .unwrap_or(true);
            if stale {
                self.index_shelf_note(&meta);
            }
            self.request_shelf_pdf(meta.id);
        }
    }

    fn index_shelf_note(&mut self, meta: &crate::library::NoteMeta) {
        if let Some(old) = self.shelf_hits.remove(&meta.id) {
            for key in old.pdf_keys {
                self.pdf_text.remove(&key);
                self.pdf_text_pending.remove(&key);
            }
        }
        let Some(note) = self.lib.load_note(meta.id) else {
            return;
        };
        let pdf_keys = note
            .pages
            .iter()
            .filter_map(|page| {
                let bg = page.background.as_ref()?;
                let pdf_page = bg.pdf_page?;
                Some(format!("{}:{}:{pdf_page}", note.id, bg.file))
            })
            .collect();
        self.shelf_hits.insert(
            meta.id,
            ShelfHit {
                updated: meta.updated,
                title: meta.title.to_lowercase(),
                body: note.search_body().to_lowercase(),
                pdf_keys,
            },
        );
    }

    fn request_shelf_pdf(&mut self, id: Uuid) {
        let Some(keys) = self.shelf_hits.get(&id).map(|hit| hit.pdf_keys.clone()) else {
            return;
        };
        for key in keys {
            if self.pdf_text.contains_key(&key) || self.pdf_text_pending.contains(&key) {
                continue;
            }
            let Some((note_file, page_s)) = key.rsplit_once(':') else {
                continue;
            };
            let Some((_, file)) = note_file.split_once(':') else {
                continue;
            };
            let Ok(page) = page_s.parse::<u32>() else {
                continue;
            };
            let path = self.lib.note_dir(id).join(file);
            let hub = self
                .pdf_text_hub
                .get_or_insert_with(crate::pdf::TextHub::new);
            if hub.request(crate::pdf::TextJob {
                key: key.clone(),
                path,
                page,
            }) {
                self.pdf_text_pending.insert(key);
            }
        }
    }

    fn request_open_pdf_text(&mut self) {
        let Some(note) = self.note.as_ref() else {
            return;
        };
        let id = note.id;
        let jobs: Vec<_> = note
            .pages
            .iter()
            .filter_map(|page| {
                let bg = page.background.as_ref()?;
                let pdf_page = bg.pdf_page?;
                let key = format!("{}:{}:{pdf_page}", id, bg.file);
                (!self.pdf_text.contains_key(&key) && !self.pdf_text_pending.contains(&key))
                    .then(|| (key, self.lib.note_dir(id).join(&bg.file), pdf_page))
            })
            .collect();
        if jobs.is_empty() {
            return;
        }
        let hub = self
            .pdf_text_hub
            .get_or_insert_with(crate::pdf::TextHub::new);
        for (key, path, page) in jobs {
            if hub.request(crate::pdf::TextJob {
                key: key.clone(),
                path,
                page,
            }) {
                self.pdf_text_pending.insert(key);
            }
        }
    }

    fn jump_to_page(&mut self, page: usize) {
        let last = self
            .note
            .as_ref()
            .map_or(0, |note| note.pages.len().saturating_sub(1));
        self.land_page = Some(page.min(last));
        self.need_fit = false;
    }

    fn shelf_hit_matches(&self, id: Uuid, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let Some(hit) = self.shelf_hits.get(&id) else {
            return self
                .lib
                .index
                .notes
                .iter()
                .any(|m| m.id == id && m.title.to_lowercase().contains(query));
        };
        if hit.title.contains(query) || hit.body.contains(query) {
            return true;
        }
        hit.pdf_keys.iter().any(|key| {
            self.pdf_text
                .get(key)
                .is_some_and(|text| text.contains(query))
        })
    }

    fn shelf_hit_snippet(&self, id: Uuid, query: &str) -> Option<String> {
        if query.is_empty() {
            return None;
        }
        let hit = self.shelf_hits.get(&id)?;
        if hit.title.contains(query) {
            return None;
        }
        search_snippet(&hit.body, query).or_else(|| {
            hit.pdf_keys.iter().find_map(|key| {
                self.pdf_text
                    .get(key)
                    .and_then(|text| search_snippet(text, query))
            })
        })
    }

    fn ui_shelf(&mut self, ctx: &Context) {
        self.handle_drops_and_paste(ctx);
        self.ingest_pdf_text(ctx);
        self.refresh_shelf_search();
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
                    .clone()
                    .into_iter()
                    .filter(|m| query.is_empty() || self.shelf_hit_matches(m.id, &query))
                    .filter(|m| !self.shelf_toss.iter().any(|t| t.id == m.id))
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
                        } else if querying {
                            Self::shelf_slot() + vec2(0.0, SNIPPET_ROW)
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
                                    let snippet = if querying {
                                        self.shelf_hit_snippet(meta.id, &query)
                                    } else {
                                        None
                                    };
                                    let action = if compact_cards {
                                        self.compact_dos(
                                            ui,
                                            &meta,
                                            selected,
                                            false,
                                            slot,
                                            snippet.as_deref(),
                                        )
                                    } else {
                                        self.notebook_cover(
                                            ui,
                                            &meta,
                                            selected,
                                            false,
                                            snippet.as_deref(),
                                        )
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

        self.ui_shelf_search(ctx);

        let fiche_ouverte = !self.lib.index.fiche_pliee;
        Area::new(Id::new("shelf-top-right"))
            .anchor(Align2::RIGHT_TOP, vec2(-16.0, 10.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                    let open = self.open_signet(ui);
                    open.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Open file")
                    });
                    if open.clicked() {
                        self.pending_open = true;
                    }
                    if self.inkwell(ui).clicked() {
                        self.new_note();
                    }
                    if self.help_signet(ui, fiche_ouverte).clicked() {
                        self.lib.index.fiche_pliee = !self.lib.index.fiche_pliee;
                        let saved = self.lib.save_index();
                        self.report_save(saved);
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

    /// Shared search field on the shelf and in PDF notes.
    fn paint_search_capsule(&self, ui: &Ui, rect: Rect, focused: bool) {
        let radius = CornerRadius::same((rect.height() * 0.5) as u8);
        if focused {
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, self.look.accent.gamma_multiply(0.85)),
                StrokeKind::Outside,
            );
        } else {
            ui.painter().rect_filled(rect, radius, self.look.desk_deep);
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, self.look.desk_edge.gamma_multiply(0.72)),
                StrokeKind::Inside,
            );
        }
    }

    fn ui_shelf_search(&mut self, ctx: &Context) {
        if !matches!(self.scene, Scene::Shelf { .. }) {
            return;
        }
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
                let width = ctx.screen_rect().width();
                let search_w = if width < 700.0 {
                    (width - 68.0).max(1.0)
                } else {
                    (width - 600.0).clamp(1.0, 420.0)
                };
                let search_h = 36.0;
                let (outer, _) = ui.allocate_exact_size(vec2(search_w, search_h), Sense::hover());
                self.paint_search_capsule(ui, outer, false);
                let inner = Rect::from_min_max(
                    pos2(outer.min.x + 16.0, outer.min.y + 6.0),
                    pos2(outer.max.x - 16.0, outer.max.y - 6.0),
                );
                let mut focused = false;
                ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                    ui.set_min_size(inner.size());
                    if let Scene::Shelf { query } = &mut self.scene {
                        let hint = self.look.fg.gamma_multiply(0.42);
                        ui.visuals_mut().override_text_color = Some(self.look.fg);
                        ui.visuals_mut().widgets.noninteractive.fg_stroke = Stroke::new(1.0, hint);
                        let te = TextEdit::singleline(query)
                            .hint_text("Search")
                            .font(self.look.mono(14.0))
                            .text_color(self.look.fg)
                            .frame(false)
                            .margin(Margin::ZERO)
                            .vertical_align(Align::Center)
                            .id(Id::new("shelf-search-field"));
                        let resp = ui.add_sized(inner.size(), te);
                        focused = resp.has_focus();
                        if focused {
                            ui.ctx().request_repaint();
                        }
                    }
                });
                if focused {
                    self.paint_search_capsule(ui, outer, true);
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

    /// Warm ochre inkwell (theme orange). Glyph is an up arrow in the lectern tray.
    fn open_signet(&self, ui: &mut Ui) -> Response {
        let size = 56.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let p = ui.painter();
        let o = self.look.orange();
        let well = if resp.hovered() {
            shade_rgb(o, 1.14)
        } else {
            o
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
    fn paint_notebook_page(&self, p: &Painter, rect: Rect, paper: &TextureHandle) -> Rect {
        self.paint_notebook_leaf(p, rect, 20.0, 18.0, paper)
    }

    fn paint_notebook_leaf(
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
        let h = if stacked { 820.0 } else { 480.0 };
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
        let paper = self.paper_tex(ui.ctx());
        let p = ui.painter_at(rect);
        let inner = self.paint_notebook_page(&p, rect, &paper);
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
            ("Paper", "M"),
            ("Trash", "Del"),
            ("Pan", "space"),
            ("Undo", "Ctrl+Z"),
            ("Save", "Ctrl+S"),
            ("Zoom", "+ −"),
        ];
        let hands: &[(&str, &str)] = &[
            ("Draw", "stylus / finger*"),
            ("Pan", "empty paper"),
            ("Highlight line", "highlighter"),
            ("Erase", "stylus button"),
            ("Undo", "two-finger tap"),
            ("Zoom", "pinch"),
            ("* Finger draw", "notebook menu"),
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
        let mut budget = u32::MAX;
        self.paint_emoji_budgeted(ctx, painter, bounds, emoji, &mut budget, false)
    }

    /// Like `paint_emoji`, but stops decoding when `budget` is spent.
    /// `small` decodes the light picker bitmap and caches it separately, so
    /// folder spines keep the full-quality texture.
    fn paint_emoji_budgeted(
        &mut self,
        ctx: &Context,
        painter: &Painter,
        bounds: Rect,
        emoji: &str,
        budget: &mut u32,
        small: bool,
    ) -> bool {
        let Some(ch) = self.emoji.key(emoji) else {
            return false;
        };
        let has = match small {
            true => self.emoji_tex_small.contains_key(&ch),
            false => self.emoji_tex.contains_key(&ch),
        };
        if !has {
            let (glyph, deferred) = self.emoji.ensure_budgeted(ch, small, budget);
            if deferred {
                ctx.request_repaint();
                return false;
            }
            let Some(g) = glyph else {
                return false;
            };
            let img = ColorImage::from_rgba_unmultiplied([g.width, g.height], &g.rgba);
            let tex = ctx.load_texture(format!("emoji-{ch}"), img, TextureOptions::LINEAR);
            let cache = if small {
                &mut self.emoji_tex_small
            } else {
                &mut self.emoji_tex
            };
            cache.insert(ch, tex);
        }
        let cache = if small {
            &self.emoji_tex_small
        } else {
            &self.emoji_tex
        };
        let Some(tex) = cache.get(&ch).cloned() else {
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

    /// Paper sheet of spine marks: iPhone category tabs, scroll the active group.
    fn ui_emoji_picker(&mut self, ctx: &Context, note_id: Uuid) {
        if self.emoji.group_chars(self.emoji_group).is_empty() {
            if let Some((group, _)) = self.emoji.sections().first() {
                self.emoji_group = *group;
            }
        }
        let mut chosen: Option<String> = None;
        // Picker glyphs decode at 40 px: several per frame fill the grid fast.
        let mut budget = 24_u32;
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
                    let tab_h = 40.0;
                    let gap = 4.0;
                    let avail = ui.available_width().max(40.0);
                    let (tab_strip, _) = ui.allocate_exact_size(vec2(avail, tab_h), Sense::hover());
                    let groups: Vec<_> = self.emoji.sections().iter().map(|(g, _)| *g).collect();
                    let n_tabs = groups.len().max(1) as f32;
                    let tab_w = (tab_strip.width() - gap * (n_tabs - 1.0).max(0.0)) / n_tabs;
                    for (i, group) in groups.iter().enumerate() {
                        let x = tab_strip.min.x + i as f32 * (tab_w + gap);
                        let cell =
                            Rect::from_min_size(pos2(x, tab_strip.min.y), vec2(tab_w, tab_h));
                        let selected = self.emoji_group == *group;
                        let resp =
                            ui.interact(cell, Id::new(("emoji-tab", *group as u8)), Sense::click());
                        if selected {
                            ui.painter().rect_filled(
                                cell.shrink(2.0),
                                CornerRadius::same(6),
                                mix_col(paper_col, ink, 0.10),
                            );
                        } else if resp.hovered() {
                            ui.painter().rect_filled(
                                cell.shrink(2.0),
                                CornerRadius::same(6),
                                mix_col(paper_col, ink, 0.05),
                            );
                        }
                        let tab = group.tab().to_string();
                        let mut tab_budget = 1_u32;
                        self.paint_emoji_budgeted(
                            ui.ctx(),
                            ui.painter(),
                            cell.shrink(8.0),
                            &tab,
                            &mut tab_budget,
                            true,
                        );
                        if resp.clicked() {
                            self.emoji_group = *group;
                        }
                    }

                    ui.add_space(6.0);
                    let catalog: Vec<char> = self.emoji.group_chars(self.emoji_group).to_vec();
                    let cols = (avail / (40.0 + gap)).floor().max(1.0) as usize;
                    let cell = (avail - gap * cols.saturating_sub(1) as f32) / cols as f32;
                    let grid_h = ui.available_height().max(1.0);
                    ui.spacing_mut().item_spacing = vec2(gap, gap);
                    ScrollArea::vertical()
                        .id_salt(("emoji-grid", note_id, self.emoji_group as u8))
                        .max_height(grid_h)
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                        .scroll_source(ScrollSource {
                            drag: true,
                            scroll_bar: false,
                            mouse_wheel: true,
                        })
                        .show_rows(ui, cell, catalog.len().div_ceil(cols).max(1), |ui, rows| {
                            ui.set_width(avail);
                            for row in rows {
                                ui.horizontal(|ui| {
                                    ui.set_width(avail);
                                    ui.spacing_mut().item_spacing = vec2(gap, 0.0);
                                    for (col_i, ch) in catalog
                                        [row * cols..((row + 1) * cols).min(catalog.len())]
                                        .iter()
                                        .enumerate()
                                    {
                                        let em = ch.to_string();
                                        let (hit, clicked) = self.stamp_cell_budgeted(
                                            ui,
                                            paper_col,
                                            ink,
                                            cell,
                                            &em,
                                            &mut budget,
                                        );
                                        if row == 0 && col_i == 0 {
                                            ui.ctx().data_mut(|d| {
                                                d.insert_temp(Id::new("emoji-first-cell"), hit);
                                            });
                                        }
                                        if clicked {
                                            chosen = Some(em);
                                        }
                                    }
                                });
                            }
                        });
                });
            });

        let dismiss = modal.should_close();
        if let Some(s) = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Paste(t) => Some(t.clone()),
                _ => None,
            })
        }) {
            let t = s.trim().to_string();
            if self.emoji.key(&t).is_some() {
                chosen = Some(t);
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
        } else if dismiss {
            self.emoji_pick = None;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stamp_cell_budgeted(
        &mut self,
        ui: &mut Ui,
        paper: Color32,
        ink: Color32,
        side: f32,
        em: &str,
        budget: &mut u32,
    ) -> (Rect, bool) {
        let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(3), mix_col(paper, ink, 0.06));
        }
        self.paint_emoji_budgeted(ui.ctx(), p, rect.shrink(5.0), em, budget, true);
        (rect, resp.clicked())
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
                                            self.compact_dos(ui, &meta, selected, true, slot, None)
                                        } else {
                                            self.notebook_cover(ui, &meta, selected, true, None)
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

    #[allow(clippy::too_many_arguments)]
    fn compact_dos(
        &mut self,
        ui: &mut Ui,
        meta: &crate::library::NoteMeta,
        selected: bool,
        in_bin: bool,
        size: Vec2,
        snippet: Option<&str>,
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
        let mut y = if snippet.is_some() {
            title.top()
        } else {
            title.center().y - galley.size().y * 0.5
        };
        let title_h = galley.size().y;
        p.galley(pos2(title.left(), y), galley, self.look.fg);
        if let Some(snippet) = snippet {
            y += title_h + 2.0;
            let mut snip = egui::text::LayoutJob::simple_singleline(
                snippet.to_string(),
                self.look.mono(11.0),
                self.look.muted,
            );
            snip.wrap.max_width = title.width();
            snip.wrap.max_rows = 1;
            snip.wrap.break_anywhere = true;
            let galley = p.layout_job(snip);
            p.galley(pos2(title.left(), y), galley, self.look.muted);
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

    #[allow(clippy::too_many_arguments)]
    fn notebook_cover(
        &mut self,
        ui: &mut Ui,
        meta: &crate::library::NoteMeta,
        selected: bool,
        in_bin: bool,
        snippet: Option<&str>,
    ) -> Option<DosAct> {
        let extra = if snippet.is_some() { SNIPPET_ROW } else { 0.0 };
        let slot = vec2(
            DOS_W + DOS_PAD * 2.0,
            DOS_PAD + PAPER_PEEK + DOS_H + TITLE_ROW + extra,
        );
        let (slot_rect, resp) = ui.allocate_exact_size(slot, Sense::click());
        let id = Id::new("notebook-cover").with(meta.id);
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
        let hit = Rect::from_min_max(face.min, pos2(face.max.x, face.max.y + TITLE_ROW + extra));
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
            if let Some(snippet) = snippet {
                painter.text(
                    pos2(title_rect.center().x, title_rect.max.y + 2.0),
                    Align2::CENTER_TOP,
                    snippet,
                    self.look.mono(11.0),
                    self.look.muted,
                );
            }
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
                if colors {
                    paint_cloth_wheel(&p, origin, r0, r1, n_cloth, hover, &self.look);
                } else {
                    let picks = [WheelPick::Color, WheelPick::Trash, WheelPick::Mark];
                    let n = picks.len();
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
                            n,
                            i,
                            tint,
                            Some((paper.id(), PAPER_TILE)),
                        ));
                        if hot {
                            p.add(wedge_stroke(origin, r0, r1, n, i, self.look.ink));
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
                    let picks = [WheelPick::Color, WheelPick::Trash, WheelPick::Mark];
                    let fills = [self.look.paper, self.look.rust(), self.look.paper];
                    for (i, pick) in picks.into_iter().enumerate() {
                        let c = slice_mid(origin, i, n, r0 + (r1 - r0) * 0.55);
                        let hot = hover == Some(pick);
                        let fill = if hot {
                            mix_col(fills[i], self.look.paper, 0.14)
                        } else {
                            fills[i]
                        };
                        let bounds = Rect::from_center_size(c, vec2(44.0, 44.0));
                        let (face, fg) = self.paint_raised_face(&p, bounds, fill, 22, hot);
                        match pick {
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

    fn sticky_at(&self, paper: Pos2) -> Option<(usize, Uuid)> {
        let note = self.note.as_ref()?;
        for (page_index, page) in note.pages.iter().enumerate().rev() {
            let local = paper - self.origin_of(page_index);
            if let Some(tx) = page
                .texts
                .iter()
                .rev()
                .find(|t| t.sticky && t.rect().contains(local))
            {
                return Some((page_index, tx.id));
            }
        }
        None
    }

    /// Long-press on empty page paper opens Pages manage mode.
    /// True means this frame belongs to the hold, not the tool.
    #[allow(clippy::too_many_arguments)]
    fn take_pages_manage_hold(
        &mut self,
        ui: &Ui,
        resp: &Response,
        rect: Rect,
        screen: &mut Pos2,
        pen_press: bool,
        primary_down: bool,
        primary_pressed: &mut bool,
        primary_released: &mut bool,
    ) -> bool {
        if self.sheet_tabs_on {
            self.pages_manage_hold = None;
            return false;
        }
        if self.tablet.snapshot().eraser {
            self.pages_manage_hold = None;
            return false;
        }
        let time = ui.input(|i| i.time);
        if *primary_pressed {
            let paper = self.camera.to_paper(*screen, rect);
            let cell = self.page_unit_at(*screen, rect);
            // A pointer press claimed by whatever floats above the canvas (⋮
            // menu rows, the coach board) must not start the pages long-hold.
            // A stylus press never goes through egui's hit test (no Pointer
            // events on lectern paper), so it arms as it always has.
            let blocked = (!pen_press && !resp.is_pointer_button_down_on())
                || self.sticky_at(paper).is_some()
                || self.hit_object(paper).is_some();
            if let Some((col, row)) = cell.filter(|_| !blocked) {
                self.pages_manage_hold = Some(PagesManageHold {
                    col,
                    row,
                    screen: *screen,
                    t0: time,
                });
                ui.ctx().request_repaint_after(Duration::from_millis(16));
                return true;
            }
            self.pages_manage_hold = None;
        }
        let Some(hold) = self.pages_manage_hold else {
            return false;
        };
        let elapsed = time - hold.t0;
        if elapsed >= HOLD_RING_DELAY {
            let progress = ((elapsed - HOLD_RING_DELAY) / (WHEEL_HOLD - HOLD_RING_DELAY))
                .clamp(0.0, 1.0) as f32;
            let progress_painter = ui.ctx().layer_painter(ui.layer_id());
            progress_painter.circle_stroke(
                *screen,
                20.0,
                Stroke::new(2.5, self.look.paper.gamma_multiply(0.75)),
            );
            let arc: Vec<Pos2> = (0..=32)
                .map(|step| {
                    let angle = -std::f32::consts::FRAC_PI_2
                        + std::f32::consts::TAU * progress * step as f32 / 32.0;
                    *screen + vec2(angle.cos(), angle.sin()) * 20.0
                })
                .collect();
            if arc.len() > 1 {
                progress_painter.add(Shape::line(arc, Stroke::new(3.5, self.look.accent)));
            }
        }
        if screen.distance(hold.screen) > 8.0 {
            let origin = hold.screen;
            self.pages_manage_hold = None;
            *screen = origin;
            *primary_pressed = true;
            return false;
        }
        if time - hold.t0 >= WHEEL_HOLD {
            self.pages_manage_hold = None;
            self.enter_pages_mode_at(Some((hold.col, hold.row)));
            self.start_page_drag(hold.col, hold.row, *screen, rect);
            return true;
        }
        if *primary_released || !primary_down {
            let origin = hold.screen;
            self.pages_manage_hold = None;
            *screen = origin;
            *primary_pressed = true;
            *primary_released = true;
            return false;
        }
        ui.ctx().request_repaint_after(Duration::from_millis(16));
        true
    }

    fn set_paper(&mut self, kind: PaperKind) {
        if let Some(n) = &mut self.note {
            if n.paper != kind {
                n.paper = kind;
                self.mark_dirty();
            }
        }
    }

    fn set_paper_tone(&mut self, tone: PaperTone) {
        if let Some(note) = &mut self.note {
            if note.paper_tone != tone {
                note.paper_tone = tone;
                self.mark_dirty();
            }
        }
    }

    /// One well of the pages-edit tray. Returns true when tapped.
    fn tray_well(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        on: bool,
        label: &str,
        tip: &str,
        icon: impl Fn(&Painter, Pos2, Color32),
    ) -> bool {
        let resp = ui.interact(rect, ui.id().with(("strip-tray", label)), Sense::click());
        resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, on, label));
        let fill = if on {
            mix_col(self.look.paper, self.look.accent, 0.34)
        } else {
            self.look.paper
        };
        let (face, fg) = self.paint_note_surface(ui, rect.shrink(2.0), &resp, fill, 10);
        if on {
            ui.painter().rect_stroke(
                face.shrink(2.0),
                8.0,
                Stroke::new(1.3, self.look.accent),
                StrokeKind::Inside,
            );
        }
        icon(ui.painter(), face.center(), fg);
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new(("strip-tray", label)), rect));
        resp.on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(tip)
            .clicked()
    }

    /// Primary exit of Pages edit: an accent key with a bold check.
    fn tray_done(&mut self, ui: &mut Ui, rect: Rect) -> bool {
        let resp = ui.interact(rect, ui.id().with(("strip-tray", "Done")), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Done"));
        let fill = mix_col(
            self.look.paper,
            self.look.accent,
            if resp.hovered() { 0.34 } else { 0.2 },
        );
        let (face, fg) = self.paint_raised_face(
            ui.painter(),
            rect.shrink(2.0),
            fill,
            10,
            resp.is_pointer_button_down_on(),
        );
        let c = face.center();
        ui.painter().add(Shape::line(
            vec![
                c + vec2(-7.5, 0.0),
                c + vec2(-2.0, 5.5),
                c + vec2(8.0, -6.0),
            ],
            Stroke::new(2.4, fg),
        ));
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new(("strip-tray", "Done")), rect));
        resp.on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Done arranging pages")
            .clicked()
    }

    /// One control for page flow: a vertical stack, two to four per row, or a
    /// single horizontal row. The last stop is the row, so orientation and
    /// density never contradict each other.
    fn tray_flow_slider(&mut self, ui: &mut Ui, rect: Rect, arrangement: Option<PageArrangement>) {
        let current = match arrangement {
            Some(PageArrangement::TwoColumns) => 2,
            Some(PageArrangement::ThreeColumns) => 3,
            Some(PageArrangement::FourColumns) => 4,
            Some(PageArrangement::Row) => 5,
            _ => 1,
        };
        let resp = ui.interact(rect, ui.id().with("strip-columns"), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Page flow"));
        let p = ui.painter().clone();
        // Recessed pill: the five stops read as one scale.
        p.rect_filled(rect, 11.0, mix_col(self.look.paper, self.look.ink, 0.05));
        p.rect_stroke(
            rect,
            11.0,
            Stroke::new(1.0, self.look.ink.gamma_multiply(0.12)),
            StrokeKind::Inside,
        );
        let span = (rect.width() - 32.0).max(32.0);
        let track_y = rect.bottom() - 9.0;
        let detent_x = |n: f32| rect.left() + 16.0 + (n - 1.0) * span / 4.0;
        let spacing = span / 4.0;
        p.line_segment(
            [pos2(detent_x(1.0), track_y), pos2(detent_x(5.0), track_y)],
            Stroke::new(2.0, self.look.ink.gamma_multiply(0.22)),
        );
        let hover = resp.hover_pos();
        let hovered_detent = hover.map(|pos| {
            (((pos.x - detent_x(1.0)) / (span / 4.0))
                .round()
                .clamp(0.0, 4.0) as u8)
                + 1
        });
        for n in 1..=5u8 {
            let node = pos2(detent_x(n as f32), track_y);
            let on = current == n;
            let name = if n == 5 {
                "all".to_owned()
            } else {
                n.to_string()
            };
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    Id::new(("strip-columns", name)),
                    Rect::from_center_size(
                        pos2(node.x, rect.center().y),
                        vec2(spacing, rect.height() - 2.0),
                    ),
                )
            });
            if hover.is_some_and(|h| (h.x - node.x).abs() <= spacing * 0.5) {
                p.circle_filled(
                    pos2(node.x, rect.center().y - 4.0),
                    (spacing * 0.36).min(15.0),
                    self.look.ink.gamma_multiply(0.07),
                );
            }
            let fg = if on {
                self.look.accent
            } else {
                self.look.ink.gamma_multiply(0.62)
            };
            if spacing >= 46.0 {
                let cell = ((spacing - 8.0) / 4.0).clamp(4.0, 7.0);
                let (cols, rows) = match n {
                    1 => (1, 3),
                    2 => (2, 2),
                    3 => (3, 2),
                    4 => (4, 2),
                    _ => (4, 1),
                };
                paint_grid_icon_sized(
                    &p,
                    pos2(node.x, rect.center().y - 4.0),
                    cols,
                    rows,
                    cell,
                    cell * 0.38,
                    fg,
                );
            } else if n == 5 {
                // Narrow: three small pages in a row still reads "all in one row".
                for dx in [-7.0, 0.0, 7.0] {
                    p.rect_stroke(
                        Rect::from_center_size(
                            pos2(node.x + dx, rect.center().y - 4.0),
                            Vec2::splat(4.6),
                        ),
                        1.0,
                        Stroke::new(1.0, fg),
                        StrokeKind::Inside,
                    );
                }
            } else {
                p.text(
                    pos2(node.x, rect.center().y - 4.0),
                    Align2::CENTER_CENTER,
                    n.to_string(),
                    self.look.mono(10.0),
                    fg,
                );
            }
            p.circle_filled(
                node,
                if on { 4.5 } else { 3.0 },
                if on {
                    self.look.accent
                } else {
                    self.look.ink.gamma_multiply(0.4)
                },
            );
        }
        let resp = resp
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text_at_pointer(match hovered_detent {
                Some(1) => "One page per row".to_owned(),
                Some(2) => "Two pages per row".to_owned(),
                Some(3) => "Three pages per row".to_owned(),
                Some(4) => "Four pages per row".to_owned(),
                Some(_) => "All pages in one row".to_owned(),
                None => "Page flow".to_owned(),
            });
        if resp.clicked() {
            let target = hovered_detent.unwrap_or(current);
            self.arrange_pages(match target {
                2 => PageArrangement::TwoColumns,
                3 => PageArrangement::ThreeColumns,
                4 => PageArrangement::FourColumns,
                5 => PageArrangement::Row,
                _ => PageArrangement::Column,
            });
        }
    }

    /// Pages edit lives on the paper: this tray replaces the whole header while
    /// arranging sheets. One row on a wide lectern — sheets, flow, paper, tone,
    /// done — and two rows when the canvas is narrow. Icon-only keys, grouped
    /// by hairlines, each explained on hover.
    fn ui_pages_edit_strip(&mut self, ctx: &Context) {
        if !self.sheet_tabs_on || !Camera::valid_viewport(self.canvas_rect) {
            return;
        }
        let canvas = self.canvas_rect;
        let arrangement = self.page_arrangement();
        let join = self
            .note
            .as_ref()
            .map(|note| note.sheet_join)
            .unwrap_or(SheetJoin::Linked);
        let tone = self
            .note
            .as_ref()
            .map(|note| note.paper_tone)
            .unwrap_or(PaperTone::Default);

        // Wide canvases get a single instrument row; below that the tray folds
        // on two rows (structure then surface) until it turns compact.
        let wide = canvas.width() >= 900.0;
        let width = if wide {
            840.0
        } else {
            (canvas.width() - 16.0).clamp(280.0, 620.0)
        };
        let compact = !wide && width < 450.0;
        let pad = if wide {
            16.0
        } else if compact {
            11.0
        } else {
            14.0
        };
        let well_w = if wide {
            46.0
        } else if compact {
            38.0
        } else {
            44.0
        };
        let well_h = if wide {
            44.0
        } else if compact {
            36.0
        } else {
            42.0
        };
        let gap = if compact { 4.0 } else { 6.0 };
        let sep = if wide {
            16.0
        } else if compact {
            12.0
        } else {
            16.0
        };
        let done_w = if wide {
            56.0
        } else if compact {
            46.0
        } else {
            52.0
        };
        let row_gap = 8.0;
        let rows_h = if wide { well_h } else { well_h * 2.0 + row_gap };
        let tray_h = pad * 2.0 + rows_h;
        let strip = Rect::from_min_size(
            pos2(
                canvas.center().x - width * 0.5,
                (canvas.bottom() - tray_h - 12.0).max(canvas.top() + 4.0),
            ),
            vec2(width, tray_h),
        );
        let row_a = strip.top() + pad;
        let row_b = row_a + well_h + row_gap;
        let surface_y = if wide { row_a } else { row_b };

        let linked = Rect::from_min_size(pos2(strip.left() + pad, row_a), vec2(well_w, well_h));
        let separate = linked.translate(vec2(well_w + gap, 0.0));
        let done = Rect::from_min_size(
            pos2(strip.right() - pad - done_w, row_a),
            vec2(done_w, well_h),
        );
        let swatch_w = if compact { 34.0 } else { 40.0 };
        let swatch_gap = if compact { 4.0 } else { 5.0 };
        let paper_w = swatch_w * PaperKind::ALL.len() as f32
            + swatch_gap * PaperKind::ALL.len().saturating_sub(1) as f32;
        // Wide: the appearance cluster hangs right of the flow control, before
        // the done key. Narrow: papers start on the left, tones on the right.
        let (tone_step, tone_right, paper_x) = if wide {
            let step = 34.0;
            let right = done.left() - sep - step * 0.5;
            let left = right - 2.0 * step - (step - 2.0) * 0.5;
            (step, right, left - sep - paper_w)
        } else {
            let x = strip.left() + pad;
            let right = strip.right() - pad - 12.0;
            let step = ((right - (x + paper_w) - 6.0) / 2.5).clamp(22.0, 34.0);
            (step, right, x)
        };
        let paper_end = paper_x + paper_w;
        let tone_left = tone_right - 2.0 * tone_step - (tone_step - 2.0) * 0.5;
        let dot_r = (tone_step * 0.3).clamp(8.0, 11.0);
        let flow_x = separate.right() + sep;
        let flow_right = if wide {
            paper_x - sep
        } else {
            done.left() - sep
        };
        let flow = Rect::from_min_size(
            pos2(flow_x, row_a),
            vec2((flow_right - flow_x).max(44.0), well_h),
        );
        let icon_scale = (well_w / 42.0).clamp(0.85, 1.2);
        let paper_tex = self.paper_tex(ctx);
        Area::new(Id::new("pages-edit-strip"))
            .fixed_pos(strip.min)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                let p = ui.painter().clone();
                p.rect_filled(
                    strip.translate(vec2(0.0, 4.0)),
                    16.0,
                    self.look.shadow.gamma_multiply(0.45),
                );
                p.rect_filled(strip, 16.0, self.look.paper);
                let grain = p.add(Shape::Noop);
                p.set(grain, paper_grain(strip.shrink(1.0), 15, &paper_tex));
                p.rect_stroke(
                    strip,
                    16.0,
                    Stroke::new(1.0, self.look.desk_edge),
                    StrokeKind::Inside,
                );
                p.rect_stroke(
                    strip.shrink(3.0),
                    13.0,
                    Stroke::new(1.0, self.look.ink.gamma_multiply(0.06)),
                    StrokeKind::Inside,
                );
                // Hairlines keep each cluster of the instrument distinct.
                // On a single row the gaps are tight enough for hairlines
                // around every cluster; on two rows the shapes separate the
                // surface row on their own.
                let mut dividers = vec![((separate.right() + flow.left()) * 0.5, row_a)];
                if wide {
                    dividers.push(((flow.right() + paper_x) * 0.5, row_a));
                    dividers.push(((paper_end + tone_left) * 0.5, row_a));
                }
                for (x, row_y) in dividers {
                    p.vline(
                        x,
                        (row_y + 3.0)..=(row_y + well_h - 3.0),
                        Stroke::new(1.0, self.look.ink.gamma_multiply(0.14)),
                    );
                }
                if self.tray_well(
                    ui,
                    linked,
                    join == SheetJoin::Linked,
                    "Linked",
                    "Linked pages — one sheet",
                    |p, c, fg| paint_join_linked(p, c, fg, icon_scale),
                ) {
                    self.set_sheet_join(SheetJoin::Linked);
                }
                if self.tray_well(
                    ui,
                    separate,
                    join == SheetJoin::Separate,
                    "Separate",
                    "Separate pages — a gutter",
                    |p, c, fg| paint_join_separate(p, c, fg, icon_scale),
                ) {
                    self.set_sheet_join(SheetJoin::Separate);
                }
                self.tray_flow_slider(ui, flow, arrangement);
                if self.tray_done(ui, done) {
                    self.leave_pages_mode();
                }
                for (index, kind) in PaperKind::ALL.into_iter().enumerate() {
                    let swatch = Rect::from_min_size(
                        pos2(paper_x + index as f32 * (swatch_w + swatch_gap), surface_y),
                        vec2(swatch_w, well_h),
                    );
                    let on = self.note.as_ref().is_some_and(|note| note.paper == kind);
                    let resp = ui.interact(
                        swatch,
                        ui.id().with(("strip-paper", kind.label())),
                        Sense::click(),
                    );
                    resp.widget_info(|| {
                        WidgetInfo::selected(WidgetType::Button, true, on, kind.label())
                    });
                    let p = ui.painter();
                    self.paint_paper_swatch(p, swatch.shrink(2.0), kind);
                    if on {
                        p.rect_stroke(
                            swatch,
                            8.0,
                            Stroke::new(1.8, self.look.accent),
                            StrokeKind::Outside,
                        );
                    } else {
                        p.rect_stroke(
                            swatch,
                            8.0,
                            Stroke::new(
                                1.0,
                                self.look.ink.gamma_multiply(if resp.hovered() {
                                    0.34
                                } else {
                                    0.18
                                }),
                            ),
                            StrokeKind::Inside,
                        );
                    }
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(Id::new(("strip-paper", kind.label())), swatch)
                    });
                    if resp
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(format!("{} paper", kind.label()))
                        .clicked()
                    {
                        self.set_paper(kind);
                    }
                }
                for (index, (name, tone_choice)) in [
                    ("White", PaperTone::White),
                    ("Sand", PaperTone::Default),
                    ("Black", PaperTone::Black),
                ]
                .into_iter()
                .enumerate()
                {
                    let cell = Rect::from_center_size(
                        pos2(
                            tone_right - (2 - index) as f32 * tone_step,
                            surface_y + well_h * 0.5,
                        ),
                        vec2(tone_step - 2.0, well_h),
                    );
                    let on = tone == tone_choice;
                    let resp =
                        ui.interact(cell, ui.id().with(("strip-tone", name)), Sense::click());
                    resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, on, name));
                    let p = ui.painter();
                    let dot = self.look.paper_surface(tone_choice);
                    if resp.hovered() && !on {
                        p.circle_stroke(
                            cell.center(),
                            dot_r + 3.0,
                            Stroke::new(1.0, self.look.ink.gamma_multiply(0.25)),
                        );
                    }
                    p.circle_filled(cell.center(), dot_r, dot);
                    p.circle_stroke(
                        cell.center(),
                        dot_r,
                        Stroke::new(1.0, self.look.ink.gamma_multiply(0.42)),
                    );
                    if on {
                        p.circle_stroke(
                            cell.center(),
                            dot_r + 3.0,
                            Stroke::new(1.6, self.look.accent),
                        );
                        paint_check(
                            p,
                            cell.center(),
                            well_glyph(dot, self.look.paper, self.look.ink),
                        );
                    }
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(Id::new(("strip-tone", name)), cell));
                    if resp
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(match tone_choice {
                            PaperTone::White => "White pages",
                            PaperTone::Default => "Sand pages",
                            PaperTone::Black => "Black pages",
                        })
                        .clicked()
                    {
                        self.set_paper_tone(tone_choice);
                    }
                }
            });
        ctx.data_mut(|d| d.insert_temp(Id::new("pages-strip-rect"), strip));
    }

    fn ui_pdf_page_number(&mut self, ctx: &Context) {
        if !self.note_has_pdf() || !Camera::valid_viewport(self.canvas_rect) {
            return;
        }
        let page_count = self.note.as_ref().map_or(0, |note| note.pages.len());
        if page_count == 0 {
            return;
        }
        let current = self.page_in_view(self.canvas_rect).min(page_count - 1) + 1;
        let visible_width = 28.0 + current.to_string().len() as f32 * 7.0;
        let number = Area::new(Id::new("pdf-page-number"))
            .fixed_pos(pos2(
                self.canvas_rect.left() + 8.0,
                self.canvas_rect.bottom() - 52.0,
            ))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                let (hit, response) =
                    ui.allocate_exact_size(vec2(visible_width.max(44.0), 44.0), Sense::click());
                ctx.data_mut(|d| d.insert_temp(Id::new("pdf-page-number-rect"), hit));
                response.widget_info(|| {
                    WidgetInfo::labeled(
                        WidgetType::Button,
                        true,
                        format!("Go to page {current} of {page_count}"),
                    )
                });
                let rect = Rect::from_center_size(hit.center(), vec2(visible_width, 26.0));
                let fill = mix_col(
                    self.look.desk,
                    self.look.paper,
                    if response.hovered() { 0.26 } else { 0.13 },
                );
                ui.painter().rect_filled(rect, 8.0, fill);
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    current.to_string(),
                    self.look.mono(13.0),
                    self.look.fg,
                );
                response
                    .on_hover_text("Go to page")
                    .on_hover_cursor(CursorIcon::PointingHand)
            });
        if number.inner.clicked() {
            self.pdf_page_target = current;
            self.pdf_page_repeat = None;
        }
        let opened = Popup::menu(&number.inner)
            .align(egui::RectAlign::TOP_START)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .frame(self.note_popup_frame().fill(self.look.paper))
            .show(|ui| {
                ui.set_width(196.0_f32.min(ctx.screen_rect().width() - 36.0).max(140.0));
                ui.horizontal(|ui| {
                    let back = self.note_action(ui, "−", 44.0, false);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(self.pdf_page_target.to_string())
                                .font(self.look.serif(24.0))
                                .color(self.look.ink),
                        );
                        ui.label(
                            RichText::new(format!("of {page_count}"))
                                .font(self.look.mono(11.0))
                                .color(self.look.ink.gamma_multiply(0.6)),
                        );
                    });
                    let next = self.note_action(ui, "+", 44.0, false);
                    let held = if let Some((direction, _, _)) = self.pdf_page_repeat {
                        ui.input(|input| input.pointer.primary_down())
                            .then_some(direction)
                    } else if back.is_pointer_button_down_on() {
                        Some(-1)
                    } else if next.is_pointer_button_down_on() {
                        Some(1)
                    } else {
                        None
                    };
                    let now = ui.input(|input| input.time);
                    let was_held = self.pdf_page_repeat.is_some();
                    let step = if let Some(direction) = held {
                        ctx.request_repaint_after(Duration::from_millis(20));
                        match &mut self.pdf_page_repeat {
                            Some((held_direction, started, next_repeat))
                                if *held_direction == direction =>
                            {
                                if now >= *next_repeat {
                                    *next_repeat = now
                                        + if now - *started >= 2.0 {
                                            0.055
                                        } else if now - *started >= 1.0 {
                                            0.11
                                        } else {
                                            0.20
                                        };
                                    Some(direction)
                                } else {
                                    None
                                }
                            }
                            _ => {
                                self.pdf_page_repeat = Some((direction, now, now + 0.45));
                                Some(direction)
                            }
                        }
                    } else {
                        self.pdf_page_repeat = None;
                        if was_held {
                            None
                        } else if back.clicked() {
                            Some(-1)
                        } else if next.clicked() {
                            Some(1)
                        } else {
                            None
                        }
                    };
                    if let Some(direction) = step {
                        self.pdf_page_target = if direction < 0 {
                            self.pdf_page_target.saturating_sub(1).max(1)
                        } else {
                            (self.pdf_page_target + 1).min(page_count)
                        };
                        self.jump_to_page(self.pdf_page_target - 1);
                    }
                });
            });
        self.pdf_page_picker_open = opened.is_some();
        if !self.pdf_page_picker_open {
            self.pdf_page_repeat = None;
        }
    }

    fn pdf_search_picker(&mut self, ui: &mut Ui, button: &Response) -> Option<Rect> {
        let focus_requested = button.ctx.data_mut(|data| {
            data.remove_temp::<bool>(Id::new("pdf-search-focus-requested"))
                .unwrap_or(false)
        });
        if button.clicked() {
            self.pdf_search_open = !self.pdf_search_open;
        }
        if !self.pdf_search_open {
            button
                .ctx
                .memory_mut(|memory| memory.surrender_focus(Id::new("pdf-search-input")));
            return None;
        }
        let width = ui.available_width().clamp(1.0, 420.0);
        let (outer, _) = ui.allocate_exact_size(vec2(width, 36.0), Sense::hover());
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("pdf-search-field-rect"), outer));
        self.paint_search_capsule(ui, outer, false);
        let show_arrows = width >= 250.0;
        let show_count = width >= 160.0;
        let mut right = outer.right() - 10.0;
        let close_rect =
            Rect::from_center_size(pos2(right - 12.0, outer.center().y), vec2(24.0, 30.0));
        right -= 28.0;
        let next_rect = show_arrows.then(|| {
            let rect =
                Rect::from_center_size(pos2(right - 12.0, outer.center().y), vec2(24.0, 30.0));
            right -= 28.0;
            rect
        });
        let previous_rect = show_arrows.then(|| {
            let rect =
                Rect::from_center_size(pos2(right - 12.0, outer.center().y), vec2(24.0, 30.0));
            right -= 28.0;
            rect
        });
        let count_right = right;
        if show_count {
            right -= 60.0;
        }
        let inner = Rect::from_min_max(
            pos2(outer.left() + 12.0, outer.top() + 6.0),
            pos2((right - 6.0).max(outer.left() + 13.0), outer.bottom() - 6.0),
        );
        let enter_step = ui
            .ctx()
            .memory(|memory| memory.has_focus(Id::new("pdf-search-input")))
            .then(|| {
                ui.input(|i| {
                    i.key_pressed(Key::Enter)
                        .then_some(if i.modifiers.shift { -1 } else { 1 })
                })
            })
            .flatten();
        let edit = ui
            .scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                ui.set_min_size(inner.size());
                let hint = self.look.fg.gamma_multiply(0.42);
                ui.visuals_mut().override_text_color = Some(self.look.fg);
                ui.visuals_mut().widgets.noninteractive.fg_stroke = Stroke::new(1.0, hint);
                ui.add_sized(
                    inner.size(),
                    TextEdit::singleline(&mut self.pdf_search_query)
                        .id(Id::new("pdf-search-input"))
                        .hint_text("Search a word or phrase…")
                        .font(self.look.mono(14.0))
                        .text_color(self.look.fg)
                        .frame(false)
                        .margin(Margin::ZERO)
                        .vertical_align(Align::Center),
                )
            })
            .inner;
        if edit.has_focus() {
            self.paint_search_capsule(ui, outer, true);
        }
        if button.clicked() || focus_requested || enter_step.is_some() {
            edit.request_focus();
        }
        self.request_open_pdf_text();
        let query = self.pdf_search_query.trim().to_lowercase();
        let matches: Vec<(usize, usize)> = self
            .note
            .as_ref()
            .map(|note| {
                note.pages
                    .iter()
                    .enumerate()
                    .filter_map(|(index, page)| {
                        let bg = page.background.as_ref()?;
                        let pdf_page = bg.pdf_page?;
                        let key = format!("{}:{}:{pdf_page}", note.id, bg.file);
                        let text = self.pdf_text.get(&key)?;
                        let count = if query.is_empty() {
                            0
                        } else {
                            text.matches(&query).count()
                        };
                        (count > 0).then_some((index, count))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let total: usize = matches.iter().map(|(_, count)| count).sum();
        let pending = !query.is_empty()
            && self.note.as_ref().is_some_and(|note| {
                note.pages.iter().any(|page| {
                    page.background.as_ref().is_some_and(|bg| {
                        bg.pdf_page.is_some_and(|pdf_page| {
                            self.pdf_text_pending
                                .contains(&format!("{}:{}:{pdf_page}", note.id, bg.file))
                        })
                    })
                })
            });
        let first =
            query != self.pdf_search_seen_query || (self.pdf_search_last_count == 0 && total > 0);
        if query != self.pdf_search_seen_query {
            self.pdf_search_match = 0;
            self.pdf_search_seen_query = query.clone();
        }
        self.pdf_search_last_count = total;
        if total > 0 {
            self.pdf_search_match = self.pdf_search_match.min(total - 1);
        } else {
            self.pdf_search_match = 0;
        }

        let close = ui.interact(close_rect, ui.id().with("pdf-search-close"), Sense::click());
        close.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Close search"));
        ui.painter().text(
            close_rect.center(),
            Align2::CENTER_CENTER,
            "×",
            self.look.mono(19.0),
            self.look.fg,
        );
        let mut step = 0_i8;
        for (rect, label, direction, name) in [
            (previous_rect, "↑", -1, "Previous match"),
            (next_rect, "↓", 1, "Next match"),
        ] {
            if let Some(rect) = rect {
                let response = ui.interact(rect, ui.id().with(name), Sense::click());
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, total > 0, name));
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    self.look.mono(17.0),
                    self.look
                        .fg
                        .gamma_multiply(if total > 0 { 1.0 } else { 0.35 }),
                );
                if response.clicked() && total > 0 {
                    step = direction;
                    edit.request_focus();
                }
            }
        }
        if show_count {
            let count = if query.is_empty() {
                "0/0".to_owned()
            } else if pending {
                format!(
                    "{}/{}+",
                    if total == 0 {
                        0
                    } else {
                        self.pdf_search_match + 1
                    },
                    total
                )
            } else {
                format!(
                    "{}/{}",
                    if total == 0 {
                        0
                    } else {
                        self.pdf_search_match + 1
                    },
                    total
                )
            };
            ui.painter().text(
                pos2(count_right - 30.0, outer.center().y),
                Align2::CENTER_CENTER,
                count,
                self.look.mono(11.0),
                self.look.fg.gamma_multiply(0.78),
            );
            ui.painter().vline(
                right + 3.0,
                outer.y_range().shrink(8.0),
                Stroke::new(1.0, self.look.desk_edge),
            );
        }
        if total > 0 {
            if let Some(direction) = enter_step {
                step = direction;
            }
        }
        if total > 0 && (first || step != 0) {
            if step > 0 {
                self.pdf_search_match = (self.pdf_search_match + 1) % total;
            } else if step < 0 {
                self.pdf_search_match = (self.pdf_search_match + total - 1) % total;
            }
            let mut index = self.pdf_search_match;
            for (page, count) in &matches {
                if index < *count {
                    if self.page_in_view(self.canvas_rect) != *page {
                        self.jump_to_page(*page);
                    }
                    break;
                }
                index -= count;
            }
        }
        let outside = ui.input(|i| {
            i.pointer.any_click()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|pos| !outer.contains(pos) && !button.rect.contains(pos))
        });
        if close.clicked() || outside {
            self.pdf_search_open = false;
            ui.memory_mut(|memory| memory.surrender_focus(edit.id));
        }
        Some(outer)
    }

    /// Close crop of the ruling. A whole page fitted in the row hides the pattern.
    fn paint_paper_swatch(&self, p: &Painter, swatch: Rect, kind: PaperKind) {
        let tone = self
            .note
            .as_ref()
            .map_or(PaperTone::Default, |note| note.paper_tone);
        let sheet = self.look.paper_surface(tone);
        p.rect_filled(swatch, 3.0, sheet);
        let painter = p.with_clip_rect(swatch.shrink(1.0));
        let rule = Stroke::new(1.05, self.look.paper_rule(tone));
        let strong = Stroke::new(1.25, self.look.paper_rule_strong(tone));
        let hair = Stroke::new(0.8, self.look.paper_rule(tone));
        let margin = Stroke::new(1.4, self.look.paper_margin(tone));
        match kind {
            PaperKind::Blank => {}
            PaperKind::Lined => {
                let x = swatch.min.x + 13.0;
                painter.line_segment([pos2(x, swatch.min.y), pos2(x, swatch.max.y)], margin);
                let mut y = swatch.min.y + 6.0;
                while y < swatch.max.y - 2.0 {
                    painter.line_segment([pos2(x + 4.0, y), pos2(swatch.max.x - 4.0, y)], rule);
                    y += 7.5;
                }
            }
            PaperKind::Grid => {
                let step = 9.0;
                let mut x = swatch.min.x + step;
                while x < swatch.max.x - 1.0 {
                    painter.line_segment(
                        [pos2(x, swatch.min.y + 2.0), pos2(x, swatch.max.y - 2.0)],
                        rule,
                    );
                    x += step;
                }
                let mut y = swatch.min.y + 5.0;
                while y < swatch.max.y - 1.0 {
                    painter.line_segment(
                        [pos2(swatch.min.x + 2.0, y), pos2(swatch.max.x - 2.0, y)],
                        rule,
                    );
                    y += step;
                }
            }
            PaperKind::Dots => {
                let dot = self.look.paper_rule_strong(tone);
                let step = 8.5;
                let mut y = swatch.min.y + 6.0;
                while y < swatch.max.y - 3.0 {
                    let mut x = swatch.min.x + 6.0;
                    while x < swatch.max.x - 3.0 {
                        painter.circle_filled(pos2(x, y), 2.0, dot);
                        x += step;
                    }
                    y += step;
                }
            }
            PaperKind::Millimetre => {
                let step = 4.0;
                let mut i = 0;
                let mut y = swatch.min.y + 2.0;
                while y < swatch.max.y - 1.0 {
                    painter.line_segment(
                        [pos2(swatch.min.x + 1.0, y), pos2(swatch.max.x - 1.0, y)],
                        if i % 5 == 0 { strong } else { hair },
                    );
                    y += step;
                    i += 1;
                }
                let mut i = 0;
                let mut x = swatch.min.x + 2.0;
                while x < swatch.max.x - 1.0 {
                    painter.line_segment(
                        [pos2(x, swatch.min.y + 1.0), pos2(x, swatch.max.y - 1.0)],
                        if i % 5 == 0 { strong } else { hair },
                    );
                    x += step;
                    i += 1;
                }
            }
            PaperKind::Cornell => {
                let cue = swatch.min.x + swatch.width() * 0.34;
                let summary = swatch.max.y - 10.0;
                let mut y = swatch.min.y + 5.0;
                while y < summary - 2.5 {
                    painter.line_segment([pos2(cue + 3.0, y), pos2(swatch.max.x - 3.0, y)], rule);
                    y += 6.5;
                }
                painter.line_segment([pos2(cue, swatch.min.y), pos2(cue, summary)], margin);
                painter.line_segment(
                    [
                        pos2(swatch.min.x + 3.0, summary),
                        pos2(swatch.max.x - 3.0, summary),
                    ],
                    margin,
                );
            }
        }
        p.rect_stroke(
            swatch,
            3.0,
            Stroke::new(1.0, self.look.ink.gamma_multiply(0.22)),
            StrokeKind::Inside,
        );
    }

    /// Light-blue action row for entering Pages edit, tinted like the tray's
    /// active wells on the paper.
    fn menu_pages_row(&self, ui: &mut Ui, label: &str) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
        if resp.gained_focus() {
            resp.scroll_to_me(Some(Align::Center));
        }
        let fill = mix_col(self.look.paper, self.look.accent, 0.34);
        let (face, fg) = self.paint_note_surface(ui, rect.shrink(2.0), &resp, fill, 12);
        paint_grid_icon(
            ui.painter(),
            pos2(face.min.x + 16.0, face.center().y),
            2,
            2,
            fg,
        );
        ui.painter().text(
            pos2(face.min.x + 32.0, face.center().y),
            Align2::LEFT_CENTER,
            label,
            self.look.mono(13.0),
            fg,
        );
        resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
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

    /// Ink-on-paper section label for the notebook's settings sheet.
    fn notebook_menu_head(&self, ui: &mut Ui, title: &str) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        ui.painter().text(
            pos2(rect.left() + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            title.to_uppercase(),
            self.look.mono(11.0),
            self.look.ink.gamma_multiply(0.72),
        );
        ui.painter().hline(
            rect.x_range().shrink(10.0),
            rect.bottom() - 1.0,
            Stroke::new(1.0, self.look.ink.gamma_multiply(0.18)),
        );
    }

    fn round_well(
        &self,
        ui: &mut Ui,
        size: f32,
        fill: Color32,
        paint: impl FnOnce(&Painter, Pos2, Color32),
    ) -> Response {
        self.round_well_with_sense(ui, size, fill, Sense::click(), paint)
    }

    fn round_well_with_sense(
        &self,
        ui: &mut Ui,
        size: f32,
        fill: Color32,
        sense: Sense,
        paint: impl FnOnce(&Painter, Pos2, Color32),
    ) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), sense);
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
            Tool::Hand => 4,  // ochre
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

    /// On a touch screen, navigation should react when the finger lands rather
    /// than waiting for its release. Mouse and keyboard retain egui's usual
    /// click semantics, so dragging across a control cannot activate it.
    fn activate_navigation_touch(ctx: &Context, response: &Response) -> bool {
        response.clicked()
            || (response.enabled()
                && response.contains_pointer()
                && !Popup::is_any_open(ctx)
                && ctx.input(|i| {
                    i.any_touches()
                        && i.pointer.primary_pressed()
                        && i.pointer
                            .interact_pos()
                            .is_some_and(|pos| response.rect.contains(pos))
                }))
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
        let note_id = self.note.as_ref().map(|n| n.id);
        if note_id != self.coach.session.note {
            self.coach.session.reset(note_id);
        }
        let compact = ctx.screen_rect().width() < 880.0;
        let title_in_menu = ctx.screen_rect().width() < 600.0;
        if !self.sheet_tabs_on {
            self.poll_image_import(ctx);
            self.handle_drops_and_paste(ctx);
        }

        if self.sheet_tabs_on {
            // Pages edit owns the canvas: the tray on the paper replaces this header.
        } else {
            TopBottomPanel::top("rule")
                .exact_height(52.0)
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
                    ui.horizontal_centered(|ui| {
                        ui.add_space(6.0);
                        let exit = self.shelf_exit_btn(ui);
                        if Self::activate_navigation_touch(ctx, &exit) {
                            self.close_desk();
                            return;
                        }
                        ctx.data_mut(|d| {
                            d.insert_temp(Id::new("header-left-end"), exit.rect.right())
                        });
                        ui.add_space(8.0);
                        let chrome = self.chrome_btn();
                        if !title_in_menu {
                            let te = TextEdit::singleline(&mut self.title_buf)
                                .font(self.look.serif(20.0))
                                .desired_width(
                                    (ui.available_width() - if compact { 104.0 } else { 272.0 })
                                        .clamp(48.0, 240.0),
                                )
                                .frame(false);
                            let title = ui.add(te);
                            ctx.data_mut(|d| {
                                d.insert_temp(Id::new("header-left-end"), title.rect.right())
                            });
                            if title.changed() {
                                if let Some(n) = &mut self.note {
                                    n.title = self.title_buf.clone();
                                    self.mark_dirty();
                                }
                            }
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(8.0);
                            let more = self.round_well(ui, chrome, self.look.paper, paint_more);
                            ctx.data_mut(|d| {
                                d.insert_temp(Id::new("notebook-menu-button-rect"), more.rect)
                            });
                            more.widget_info(|| {
                                WidgetInfo::labeled(
                                    WidgetType::Button,
                                    ui.is_enabled(),
                                    "Notebook menu",
                                )
                            });
                            // On narrow windows, leave room for the PDF search field.
                            if ctx.screen_rect().width() >= 480.0 {
                                let full = self.round_well(
                                    ui,
                                    chrome,
                                    self.look.paper,
                                    paint_fullscreen_icon,
                                );
                                ctx.data_mut(|d| {
                                    d.insert_temp(Id::new("fullscreen-button-rect"), full.rect)
                                });
                                full.widget_info(|| {
                                    WidgetInfo::labeled(
                                        WidgetType::Button,
                                        ui.is_enabled(),
                                        "Fit page to window",
                                    )
                                });
                                if full.clicked() {
                                    self.fit_to_screen();
                                }
                            }
                            let right_cluster_left = more.rect.left();
                            let mut act = None;
                            // The right-hand tool case occupies the same vertical strip as
                            // the default menu. Keep this sheet entirely beside it.
                            let right_dock = (self.dock_layout_edge() == DockEdge::Right)
                                .then(|| ctx.data(|d| d.get_temp::<Rect>(Id::new("dock-rect"))))
                                .flatten();
                            let menu_right = right_dock
                                .map(|dock| dock.left() - 10.0)
                                .unwrap_or(more.rect.right());
                            let menu_width =
                                (menu_right - ctx.screen_rect().left() - 32.0).clamp(120.0, 312.0);
                            let mut popup = Popup::menu(&more)
                                .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                                .frame(self.note_popup_frame().fill(self.look.paper));
                            if right_dock.is_some() {
                                popup = popup
                                    .at_position(pos2(menu_right, more.rect.bottom()))
                                    .align(egui::RectAlign::BOTTOM_END)
                                    .align_alternatives(&[]);
                            }
                            let menu = popup.show(|ui| {
                                ui.set_width(menu_width);
                                ui.set_max_height((ctx.screen_rect().height() - 84.0).max(88.0));
                                ui.spacing_mut().item_spacing = vec2(2.0, 3.0);
                                ScrollArea::vertical()
                                    .max_height((ctx.screen_rect().height() - 84.0).max(88.0))
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        if title_in_menu {
                                            self.notebook_menu_head(ui, "Notebook title");
                                            if ui
                                                .add(
                                                    TextEdit::singleline(&mut self.title_buf)
                                                        .font(self.look.serif(20.0)),
                                                )
                                                .changed()
                                            {
                                                if let Some(n) = &mut self.note {
                                                    n.title = self.title_buf.clone();
                                                    self.mark_dirty();
                                                }
                                            }
                                        }
                                        self.notebook_menu_head(ui, "Pages");
                                        if self.menu_pages_row(ui, "Edit pages") {
                                            act = Some(MoreAct::Pages);
                                        }
                                        self.notebook_menu_head(ui, "Tools");
                                        if self.menu_row(
                                            ui,
                                            "Draw with finger",
                                            "",
                                            self.lib.index.finger_draw,
                                            false,
                                        ) {
                                            act = Some(MoreAct::FingerDraw);
                                        }
                                        if self.menu_row(
                                            ui,
                                            "Stroke eraser",
                                            "",
                                            self.last_eraser == Tool::EraserStroke,
                                            false,
                                        ) {
                                            act = Some(MoreAct::EraseStroke);
                                        }
                                        if self.menu_row(
                                            ui,
                                            "Area eraser",
                                            "",
                                            self.last_eraser == Tool::EraserArea,
                                            false,
                                        ) {
                                            act = Some(MoreAct::EraseArea);
                                        }
                                    });
                            });
                            if let Some(menu) = menu {
                                ctx.data_mut(|d| {
                                    d.insert_temp(Id::new("notebook-menu-rect"), menu.response.rect)
                                });
                            }
                            match act {
                                Some(MoreAct::Pages) => self.toggle_pages_mode(),
                                Some(MoreAct::FingerDraw) => {
                                    self.lib.index.finger_draw = !self.lib.index.finger_draw;
                                    let saved = self.lib.save_index();
                                    self.report_save(saved);
                                    self.reset_canvas_gesture();
                                }
                                Some(MoreAct::EraseStroke) => {
                                    self.set_eraser_kind(Tool::EraserStroke)
                                }
                                Some(MoreAct::EraseArea) => self.set_eraser_kind(Tool::EraserArea),
                                None => {}
                            }
                            let mut leftmost = right_cluster_left;
                            if self.note_has_pdf() {
                                let search =
                                    self.round_well(ui, chrome, self.look.paper, paint_search_icon);
                                ctx.data_mut(|d| {
                                    d.insert_temp(Id::new("pdf-search-button-rect"), search.rect)
                                });
                                search.widget_info(|| {
                                    WidgetInfo::labeled(
                                        WidgetType::Button,
                                        ui.is_enabled(),
                                        "Find in PDF",
                                    )
                                });
                                let field = self.pdf_search_picker(ui, &search);
                                leftmost = leftmost.min(field.unwrap_or(search.rect).left());
                            }
                            ctx.data_mut(|d| {
                                d.insert_temp(Id::new("header-right-start"), leftmost)
                            });
                        });
                    });
                    let left_end = ctx
                        .data(|d| d.get_temp::<f32>(Id::new("header-left-end")))
                        .unwrap_or(bar.left());
                    let right_start = ctx
                        .data(|d| d.get_temp::<f32>(Id::new("header-right-start")))
                        .unwrap_or(bar.right());
                    let center_x = ctx.screen_rect().center().x;
                    let free_half_width = (center_x - left_end - 8.0)
                        .min(right_start - center_x - 8.0)
                        .max(0.0);
                    let meter_width = (2.0 * free_half_width).clamp(0.0, 320.0);
                    let meter_rect = Rect::from_center_size(
                        pos2(center_x, bar.center().y),
                        vec2(meter_width, 44.0),
                    );
                    ctx.data_mut(|d| d.insert_temp(Id::new("coach-meter-rect"), meter_rect));
                    if self.coach_enabled() && meter_width >= 32.0 {
                        self.ui_coach_meter(ui, meter_rect);
                    }
                });
        }

        CentralPanel::default()
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                self.ui_canvas(ui);
            });

        self.ui_pdf_page_number(ctx);
        self.ui_pages_edit_strip(ctx);

        if !self.sheet_tabs_on {
            self.ui_floating_dock(ctx);
            self.ui_coach_badge(ctx);
            self.ui_ink_panel(ctx);
        }
        self.ui_selection_actions(ctx);
        if let Some((page, local)) = self.pending_image.take() {
            if self.image_import.is_none() {
                self.pick_image(ctx, page, local, None);
            }
        }
        self.ui_export_format_picker(ctx);
        self.ui_coach(ctx);
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
        self.steer_carried_dock(ctx);
        if self.dock_slide.is_some() {
            self.step_dock_slide(ctx, bounds);
        }
        self.step_dock_fold(ctx);
        let vertical = self.dock_layout_edge().vertical();
        let tools_limit = self.dock_tools_limit(vertical, bounds);
        let show_tools = tools_limit > 2.0;
        let tools_fade = (1.0 - self.dock_fold).clamp(0.0, 1.0);
        // One area per orientation. Top and bottom share a size; left and right share
        // the other. A fresh id per edge would fade in and hide for a sizing pass
        // every time the case changed side.
        let orient = u8::from(vertical);
        let mut area = Area::new(Id::new(("cahier-dock", orient)))
            .order(Order::Foreground)
            .interactable(true)
            .fade_in(false)
            .constrain_to(bounds);
        if self.dock_slide.is_some_and(|slide| slide.arrived) && self.dock_fold > 0.0 {
            // Area::anchor uses last frame's size. During expansion that would
            // trail the center by half the newly revealed tools on every frame.
            let mut size = self.dock_pill_size(vertical);
            if show_tools {
                if vertical {
                    size.y += tools_limit + 2.0;
                } else {
                    size.x += tools_limit + 3.0;
                }
            }
            area = area
                .fixed_pos(dock_rest_min(self.dock_edge, size, bounds))
                .constrain(false);
        } else if let Some(pos) = self
            .dock_float
            .filter(|_| !self.dock_slide.is_some_and(|s| s.arrived))
        {
            // The first frame of an orientation has no saved size. Give it the
            // guess so the screen clamp does not shove the case inward.
            let size = self.dock_pill_size(vertical);
            area = area.fixed_pos(pos).default_size(size).constrain(false);
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
                            if show_tools {
                                ui.multiply_opacity(tools_fade);
                                ScrollArea::vertical()
                                    .id_salt("dock-tools-vertical")
                                    .min_scrolled_height(0.0)
                                    .scroll_source(ScrollSource {
                                        drag: false,
                                        ..Default::default()
                                    })
                                    .max_height(tools_limit.max(1.0))
                                    .max_width(44.0)
                                    .auto_shrink([true, true])
                                    .scroll_bar_visibility(if self.dock_fold > 0.04 {
                                        ScrollBarVisibility::AlwaysHidden
                                    } else {
                                        ScrollBarVisibility::VisibleWhenNeeded
                                    })
                                    .show(ui, |ui| {
                                        // The scroll child otherwise inherits the whole window width,
                                        // which shifts the centered tool wells away from their case.
                                        ui.set_width(self.slot());
                                        ui.add_enabled_ui(!self.dock_held, |ui| {
                                            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                                                self.dock_inner(ui, true);
                                            });
                                        });
                                    });
                            }
                        });
                    } else {
                        ui.spacing_mut().item_spacing = vec2(3.0, 0.0);
                        ui.horizontal(|ui| {
                            self.dock_handle(ui, false);
                            if show_tools {
                                ui.multiply_opacity(tools_fade);
                                ScrollArea::horizontal()
                                    .id_salt("dock-tools-horizontal")
                                    .min_scrolled_width(0.0)
                                    .scroll_source(ScrollSource {
                                        drag: false,
                                        ..Default::default()
                                    })
                                    .max_width(tools_limit.max(1.0))
                                    .max_height(44.0)
                                    .auto_shrink([true, true])
                                    .scroll_bar_visibility(if self.dock_fold > 0.04 {
                                        ScrollBarVisibility::AlwaysHidden
                                    } else {
                                        ScrollBarVisibility::VisibleWhenNeeded
                                    })
                                    .show(ui, |ui| {
                                        // Keep the horizontal scroller's cross-axis the size of one
                                        // well. Its unconstrained child Ui would otherwise vertically
                                        // center the row in the full available canvas.
                                        ui.set_height(self.slot());
                                        ui.add_enabled_ui(!self.dock_held, |ui| {
                                            ui.with_layout(
                                                Layout::left_to_right(Align::Center),
                                                |ui| {
                                                    self.dock_inner(ui, false);
                                                },
                                            );
                                        });
                                    });
                            }
                        });
                    }
                });
        });
        let rect = inner.response.rect;
        ctx.data_mut(|d| d.insert_temp(Id::new("dock-rect"), rect));
        self.note_dock_pose(rect);
    }

    fn ink_panel_open(&self) -> bool {
        self.tin_open || self.nib_open
    }

    fn set_ink_panel(&mut self, open: bool) {
        self.tin_open = open;
        self.nib_open = open;
        if !open {
            self.color_mixer_open = false;
            self.dock_tool_drag = None;
        }
    }

    fn clear_ink_panel_rects(ctx: &Context) {
        ctx.data_mut(|d| {
            d.remove::<Rect>(Id::new("tin-rect"));
            d.remove::<Rect>(Id::new("tin-wheel"));
            d.remove::<Rect>(Id::new("nib-rect"));
            d.remove::<Rect>(Id::new("nib-track"));
            d.remove::<Rect>(Id::new("dock-tool-storage-rect"));
        });
    }

    /// The ink case shares the dock's cloth, pigment wells and finger-sized controls.
    fn ui_ink_panel(&mut self, ctx: &Context) {
        if !self.ink_panel_open() || self.dock_case_closed() {
            Self::clear_ink_panel_rects(ctx);
            if !ctx.input(|i| i.pointer.any_down()) {
                self.dock_tool_drag = None;
            }
            return;
        }
        let bounds = note_controls_bounds(ctx);
        let dock = ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap_or(Rect::from_center_size(bounds.center(), vec2(48.0, 48.0)));
        let wanted = vec2(226.0, if self.color_mixer_open { 556.0 } else { 492.0 });
        let place = accessory_rect(
            bounds,
            &[dock],
            wanted,
            vec2(226.0, 44.0),
            self.dock_layout_edge(),
        );
        let inner = Area::new(Id::new("inkarchy-tin"))
            .order(Order::Foreground)
            .fixed_pos(place.min)
            .fade_in(false)
            .constrain_to(bounds)
            .show(ctx, |ui| {
                self.note_popup_frame().inner_margin(12).show(ui, |ui| {
                    ui.set_width(200.0);
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    ScrollArea::vertical()
                        .id_salt("ink-case-scroll")
                        .max_height((place.height() - 26.0).max(44.0))
                        .auto_shrink([false, true])
                        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                        .scroll_source(ScrollSource {
                            drag: false,
                            ..Default::default()
                        })
                        .show(ui, |ui| {
                            if self.color_mixer_open {
                                ui.vertical_centered(|ui| self.ink_wheel_ui(ui, TIN_SIDE));
                                ui.horizontal(|ui| {
                                    let add = self.note_action(ui, "Add color", 124.0, false);
                                    ui.ctx().data_mut(|d| {
                                        d.insert_temp(Id::new("ink-save-color"), add.rect)
                                    });
                                    if add.clicked() {
                                        self.save_ink_swatch();
                                        self.color_mixer_open = false;
                                    }
                                    let back =
                                        self.round_well(ui, 44.0, self.look.paper, paint_back);
                                    if back.on_hover_text("Back to colors").clicked() {
                                        self.color_mixer_open = false;
                                    }
                                });
                            } else {
                                ui.ctx().data_mut(|d| {
                                    d.remove::<Rect>(Id::new("tin-wheel"));
                                });
                                self.ink_swatches_ui(ui);
                            }
                            self.nib_slip_ui(ui, vec2(200.0, 96.0));
                            self.dock_tool_storage_ui(ui, vec2(200.0, 204.0));
                        });
                });
            });
        ctx.data_mut(|d| d.insert_temp(Id::new("tin-rect"), inner.response.rect));
        if let (Some(tool), Some(pointer)) = (self.dock_tool_drag, ctx.pointer_latest_pos()) {
            let painter =
                ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("stored-tool-drag")));
            let center = pointer + vec2(0.0, -28.0);
            painter.circle_filled(center, 21.0, self.tool_well_fill(tool, true));
            painter.circle_stroke(center, 22.0, Stroke::new(2.0, self.look.paper));
            paint_tool_icon(&painter, center, self.look.ink, tool);
            if self.parked_tools.contains(&tool) {
                painter.rect_stroke(
                    dock.expand(2.0),
                    30.0,
                    Stroke::new(2.0, self.look.accent),
                    StrokeKind::Outside,
                );
            }
            ctx.set_cursor_icon(CursorIcon::Grabbing);
        }
        self.resolve_dock_tool_drop(ctx, dock);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            if let Some(p) = ctx.pointer_latest_pos() {
                if !inner.response.rect.expand(4.0).contains(p) && !dock.expand(6.0).contains(p) {
                    self.set_ink_panel(false);
                }
            }
        }
    }

    fn ink_swatches_ui(&mut self, ui: &mut Ui) {
        let mut colors = self.look.inks.clone();
        for rgb in &self.lib.index.custom_inks {
            let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            if !colors.contains(&color) {
                colors.push(color);
            }
        }
        ScrollArea::vertical()
            .id_salt("ink-swatches-scroll")
            .max_height(148.0)
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                for row in 0..(colors.len() + 1).div_ceil(4) {
                    ui.horizontal(|ui| {
                        for col in 0..4 {
                            let i = row * 4 + col;
                            if let Some(&color) = colors.get(i) {
                                let response = self.round_well(ui, 44.0, color, |_, _, _| {});
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(Id::new(("ink-swatch", i)), response.rect)
                                });
                                if color == self.ink {
                                    ui.painter().circle_stroke(
                                        response.rect.center(),
                                        21.0,
                                        Stroke::new(2.0, self.look.paper),
                                    );
                                }
                                let label =
                                    format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b());
                                response.widget_info(|| {
                                    WidgetInfo::selected(
                                        WidgetType::Button,
                                        true,
                                        color == self.ink,
                                        &label,
                                    )
                                });
                                if response.on_hover_text(label).clicked() {
                                    self.pick_free_ink(color);
                                }
                            } else if i == colors.len() {
                                let response =
                                    self.round_well(ui, 44.0, self.look.paper, |p, c, fg| {
                                        p.line_segment(
                                            [c - vec2(7.0, 0.0), c + vec2(7.0, 0.0)],
                                            Stroke::new(1.8, fg),
                                        );
                                        p.line_segment(
                                            [c - vec2(0.0, 7.0), c + vec2(0.0, 7.0)],
                                            Stroke::new(1.8, fg),
                                        );
                                    });
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(Id::new("ink-add-color"), response.rect)
                                });
                                response.widget_info(|| {
                                    WidgetInfo::labeled(
                                        WidgetType::Button,
                                        ui.is_enabled(),
                                        "Mix a new color",
                                    )
                                });
                                if response.on_hover_text("Mix a new color").clicked() {
                                    self.color_mixer_open = true;
                                }
                            }
                        }
                    });
                }
            });
    }

    fn save_ink_swatch(&mut self) {
        let rgb = [self.ink.r(), self.ink.g(), self.ink.b()];
        if !self.look.inks.contains(&self.ink) && !self.lib.index.custom_inks.contains(&rgb) {
            self.lib.index.custom_inks.push(rgb);
            let saved = self.lib.save_index();
            self.report_save(saved);
        }
    }

    fn dock_tool_storage_ui(&mut self, ui: &mut Ui, size: Vec2) {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        if self
            .dock_tool_drag
            .is_some_and(|tool| !self.parked_tools.contains(&tool))
        {
            ui.scroll_to_rect(rect, Some(Align::Max));
        }
        let visible = rect.intersect(ui.clip_rect());
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("dock-tool-storage-rect"), visible));
        let over = self.dock_tool_drag.is_some()
            && ui
                .ctx()
                .pointer_latest_pos()
                .is_some_and(|p| visible.contains(p));
        ui.painter().rect_filled(
            rect,
            18.0,
            mix_col(self.note_case_color(), self.look.shadow, 0.22),
        );
        if over {
            ui.painter().rect_stroke(
                rect,
                18.0,
                Stroke::new(2.0, self.look.accent),
                StrokeKind::Inside,
            );
        }
        ui.painter().text(
            pos2(rect.center().x, rect.top() + 12.0),
            Align2::CENTER_CENTER,
            if over {
                "Release to store"
            } else {
                "Tool storage"
            },
            self.look.mono(10.0),
            self.look.paper.gamma_multiply(0.72),
        );
        for (i, tool) in PARKABLE_TOOLS.into_iter().enumerate() {
            let slot = Rect::from_min_size(
                rect.min + vec2(12.0 + (i % 3) as f32 * 64.0, 24.0 + (i / 3) as f32 * 44.0),
                vec2(44.0, 44.0),
            );
            if self.parked_tools.contains(&tool) {
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .id_salt(("stored-tool", tool))
                        .max_rect(slot),
                );
                let response = self.paint_tool_well(&mut child, tool, false);
                if response
                    .on_hover_text("Drag to the toolbar, or tap to restore")
                    .clicked()
                {
                    self.parked_tools.remove(&tool);
                    self.save_parked_tools();
                }
            } else {
                ui.painter().circle_stroke(
                    slot.center(),
                    17.0,
                    Stroke::new(1.0, self.look.paper.gamma_multiply(0.15)),
                );
            }
        }
    }

    fn save_parked_tools(&mut self) {
        self.lib.index.parked_tools = PARKABLE_TOOLS
            .into_iter()
            .filter(|tool| self.parked_tools.contains(tool))
            .collect();
        let saved = self.lib.save_index();
        self.report_save(saved);
    }

    fn resolve_dock_tool_drop(&mut self, ctx: &Context, dock: Rect) {
        if !ctx.input(|i| i.pointer.any_released()) {
            return;
        }
        let Some(tool) = self.dock_tool_drag.take() else {
            return;
        };
        if ctx.input(|i| {
            i.events.iter().any(|event| {
                matches!(
                    event,
                    Event::Touch {
                        phase: TouchPhase::Cancel,
                        ..
                    }
                )
            })
        }) {
            return;
        }
        // Touch release includes PointerGone; its end position still identifies the drop.
        let pointer = ctx.input(|i| {
            i.pointer.interact_pos().or_else(|| {
                i.events.iter().find_map(|event| {
                    if let Event::Touch {
                        phase: TouchPhase::End,
                        pos,
                        ..
                    } = event
                    {
                        Some(*pos)
                    } else {
                        None
                    }
                })
            })
        });
        let Some(pointer) = pointer else {
            return;
        };
        let in_storage = ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-tool-storage-rect")))
            .is_some_and(|rect| rect.contains(pointer));
        let changed = if in_storage {
            self.parked_tools.insert(tool)
        } else if dock.contains(pointer) {
            self.parked_tools.remove(&tool)
        } else {
            false
        };
        if changed {
            if in_storage && tool == self.tool {
                self.tool = self.last_ink;
            }
            self.save_parked_tools();
        }
    }

    fn ink_wheel_ui(&mut self, ui: &mut Ui, side: f32) {
        let (wheel, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::Slider, ui.is_enabled(), "Ink color"));
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("tin-wheel"), wheel));
        let geom = ink_wheel_geom(wheel);
        let p = ui.painter();
        // The hue ring is the outer edge. No plate and no stroke around it.
        paint_disc_shadow(p, geom.center, geom.ring_outer, self.look.shadow);
        paint_hue_ring(p, geom.center, geom.ring_inner, geom.ring_outer);
        p.circle_filled(geom.center, geom.ring_inner - 0.4, self.look.paper);
        paint_sv_disk(p, geom.center, geom.sv_r, self.tin_hue);
        p.circle_stroke(
            geom.center,
            geom.sv_r - 0.6,
            Stroke::new(1.15, Color32::from_black_alpha(36)),
        );
        let (_, s0, v0) = rgb_to_hsv(self.ink);
        let mut cur = geom.sv_pos(s0, v0);
        let d = cur.distance(geom.center);
        if d > geom.sv_r && d > 0.5 {
            cur = geom.center + (cur - geom.center) * (geom.sv_r / d);
        }
        p.circle_filled(cur, 7.2, self.look.paper);
        p.circle_filled(cur, 4.6, self.ink);
        p.circle_stroke(cur, 7.2, Stroke::new(1.2, Color32::from_black_alpha(40)));
        let handle = geom.hue_pos(self.tin_hue);
        let hue = hsv_to_rgb(self.tin_hue, 1.0, 1.0);
        p.circle_filled(handle, 7.4, self.look.paper);
        p.circle_filled(handle, 5.0, hue);
        let zone_id = Id::new("tin-drag-zone");
        if let Some(pointer) = resp.interact_pointer_pos() {
            if resp.drag_started() || resp.clicked() {
                let dist = pointer.distance(geom.center);
                let zone = if dist <= geom.sv_r + 2.0 {
                    0_u8
                } else if dist <= geom.ring_outer + 10.0 {
                    1
                } else {
                    2
                };
                ui.ctx().data_mut(|d| d.insert_temp(zone_id, zone));
                self.apply_tin_pointer(&geom, pointer, zone);
            } else if resp.dragged() {
                let zone = ui.ctx().data(|d| d.get_temp::<u8>(zone_id)).unwrap_or(2);
                self.apply_tin_pointer(&geom, pointer, zone);
            }
        }
        if resp.drag_stopped() {
            ui.ctx().data_mut(|d| d.remove::<u8>(zone_id));
        }
    }

    fn nib_slip_ui(&mut self, ui: &mut Ui, size: Vec2) {
        let (slip, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("nib-rect"), slip));
        let presets = [1.0, 2.5, 6.0, 12.0];
        for (i, width) in presets.into_iter().enumerate() {
            let slot = Rect::from_min_size(slip.min + vec2(i as f32 * 52.0, 0.0), vec2(44.0, 44.0));
            let mut child =
                ui.new_child(UiBuilder::new().id_salt(("nib-preset", i)).max_rect(slot));
            let response = self.round_well(&mut child, 44.0, self.look.paper, |p, c, fg| {
                p.line_segment(
                    [c - vec2(8.0, 0.0), c + vec2(8.0, 0.0)],
                    Stroke::new(width, fg),
                );
            });
            if (self.width - width).abs() < 0.05 {
                ui.painter().circle_stroke(
                    response.rect.center(),
                    21.0,
                    Stroke::new(2.0, self.look.accent),
                );
            }
            response.widget_info(|| {
                WidgetInfo::selected(
                    WidgetType::Button,
                    ui.is_enabled(),
                    (self.width - width).abs() < 0.05,
                    format!("Stroke width {width:.1} px"),
                )
            });
            if response.on_hover_text(format!("{width:.1} px")).clicked() {
                self.set_nib_width(width);
            }
        }
        let hit = Rect::from_min_max(slip.min + vec2(0.0, 48.0), slip.max);
        let track = Rect::from_center_size(
            pos2(slip.center().x - 22.0, hit.center().y),
            vec2(slip.width() - 72.0, 4.0),
        );
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("nib-track"), track));
        let response = ui.interact(hit, ui.id().with("nib-adjust"), Sense::click_and_drag());
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Slider, ui.is_enabled(), "Stroke width")
        });
        if let Some(pointer) = response.interact_pointer_pos() {
            if response.dragged() || response.clicked() {
                let width = width_from_nib_t(nib_track_t(track, pointer));
                self.set_nib_width((width * 10.0).round() / 10.0);
            }
        }
        if response.has_focus() {
            let delta = ui.input(|i| {
                f32::from(i.key_pressed(Key::ArrowRight)) - f32::from(i.key_pressed(Key::ArrowLeft))
            });
            if delta != 0.0 {
                self.set_nib_width((self.width + delta * 0.1).clamp(NIB_MIN, NIB_MAX));
            }
        }
        let p = ui.painter();
        let handle = nib_handle_pos(track, nib_t(self.width));
        p.line_segment(
            [track.left_center(), track.right_center()],
            Stroke::new(3.0, self.look.paper.gamma_multiply(0.18)),
        );
        p.line_segment(
            [track.left_center(), handle],
            Stroke::new(3.0, self.look.accent),
        );
        p.circle_filled(handle, 8.0, self.look.paper);
        p.circle_filled(handle, 3.0, self.look.accent);
        p.text(
            pos2(slip.right(), hit.center().y),
            Align2::RIGHT_CENTER,
            format!("{:.1}", self.width),
            self.look.mono(12.0),
            self.look.paper,
        );
    }

    fn set_nib_width(&mut self, width: f32) {
        self.width = width.clamp(0.8, 48.0);
        if self.tool.is_ink() {
            self.last_ink_width = self.width;
        }
    }

    fn apply_tin_pointer(&mut self, geom: &InkWheelGeom, p: Pos2, zone: u8) {
        match zone {
            0 => {
                let s = ((p.x - geom.center.x) / geom.sv_r * 0.5 + 0.5).clamp(0.0, 1.0);
                let v = (0.5 - (p.y - geom.center.y) / (2.0 * geom.sv_r)).clamp(0.0, 1.0);
                self.pick_free_ink(hsv_to_rgb(self.tin_hue, s, v));
            }
            1 => {
                let ang = (p.y - geom.center.y).atan2(p.x - geom.center.x);
                self.tin_hue = (ang / std::f32::consts::TAU).rem_euclid(1.0);
                let (_, s, v) = rgb_to_hsv(self.ink);
                self.pick_free_ink(hsv_to_rgb(self.tin_hue, s.max(0.12), v.max(0.18)));
            }
            _ => {}
        }
    }

    fn dock_layout_edge(&self) -> DockEdge {
        self.dock_drag_edge.unwrap_or(self.dock_edge)
    }

    fn dock_case_closed(&self) -> bool {
        self.dock_held || self.dock_fold > 0.12
    }

    fn dock_pill_size(&self, vertical: bool) -> Vec2 {
        let s = self.slot();
        if vertical {
            vec2(s + 14.0, s + 18.0)
        } else {
            vec2(s + 18.0, s + 14.0)
        }
    }

    fn dock_tools_limit(&self, vertical: bool, bounds: Rect) -> f32 {
        let cap = if vertical {
            (bounds.height() - 70.0).max(44.0)
        } else {
            (bounds.width() - 70.0).max(44.0)
        };
        if self.dock_fold <= 0.0 {
            return cap;
        }
        let pill = self.dock_pill_size(vertical);
        let open = self.dock_size.unwrap_or(pill);
        let open_tools = if vertical {
            (open.y - pill.y - 2.0).max(0.0)
        } else {
            (open.x - pill.x - 3.0).max(0.0)
        };
        // The viewport follows the fold continuously down to the grip size.
        let close = self.dock_fold.clamp(0.0, 1.0);
        open_tools.min(cap) * (1.0 - close)
    }

    /// Spring the case closed onto the grip while held, open after release.
    fn step_dock_fold(&mut self, ctx: &Context) {
        let traveling = self.dock_slide.is_some_and(|slide| !slide.arrived);
        let target = if self.dock_held || traveling {
            1.0
        } else {
            0.0
        };
        let settled = (self.dock_fold - target).abs() < 0.002 && self.dock_fold_vel.abs() < 0.05;
        if settled {
            self.dock_fold = target;
            self.dock_fold_vel = 0.0;
            return;
        }
        let dt = ctx.input(|i| i.stable_dt).clamp(1.0 / 240.0, 0.05);
        let offset = self.dock_fold - target;
        let c = self.dock_fold_vel + DOCK_FOLD_OMEGA * offset;
        let decay = (-DOCK_FOLD_OMEGA * dt).exp();
        self.dock_fold = (target + (offset + c * dt) * decay).clamp(0.0, 1.0);
        self.dock_fold_vel = (self.dock_fold_vel - DOCK_FOLD_OMEGA * c * dt) * decay;
        ctx.request_repaint();
    }

    /// Spring the case into the edge it was released toward.
    ///
    /// egui has no reduced-motion flag. The slide is one short spring and a new
    /// grab cancels it from wherever the case is.
    fn step_dock_slide(&mut self, ctx: &Context, bounds: Rect) {
        let Some(slide) = self.dock_slide else {
            return;
        };
        if slide.arrived {
            if self.dock_fold == 0.0 {
                self.dock_slide = None;
                self.dock_float = None;
                self.dock_drag_edge = None;
                self.dock_vel = Vec2::ZERO;
            } else {
                ctx.request_repaint();
            }
            return;
        }
        let dt = ctx.input(|i| i.stable_dt).clamp(1.0 / 240.0, 0.05);
        // Travel as a closed case to the edge's center. Expansion uses that
        // same edge anchor, so changing length never changes the destination.
        let size = self.dock_pill_size(self.dock_edge.vertical());
        let target = dock_rest_min(self.dock_edge, size, bounds);
        let offset = self.dock_float.unwrap_or(target) - target;
        let c = slide.vel + offset * DOCK_SPRING_OMEGA;
        let decay = (-DOCK_SPRING_OMEGA * dt).exp();
        let pos = target + (offset + c * dt) * decay;
        let vel = (slide.vel - c * DOCK_SPRING_OMEGA * dt) * decay;
        let home = slide.shown && pos.distance(target) < 0.6 && vel.length() < 8.0;
        if home {
            self.dock_float = Some(target);
            self.dock_slide = Some(DockSlide {
                vel: Vec2::ZERO,
                arrived: true,
                ..slide
            });
            ctx.request_repaint();
        } else {
            self.dock_float = Some(pos);
            self.dock_slide = Some(DockSlide {
                vel,
                shown: slide.shown,
                arrived: slide.arrived,
            });
            ctx.request_repaint();
        }
    }

    /// Remember the expanded size only; partial layouts must not alter the target.
    fn note_dock_pose(&mut self, rect: Rect) {
        if let Some(slide) = &mut self.dock_slide {
            slide.shown = true;
        } else if !self.dock_held && self.dock_fold == 0.0 {
            self.dock_size = Some(rect.size());
        }
    }

    fn dock_handle(&mut self, ui: &mut Ui, vertical: bool) {
        let grip = self.dock_grip(ui, vertical);
        let grab_now = !self.dock_held && (grip.drag_started() || grip.is_pointer_button_down_on());
        if grab_now {
            let pointer = grip.interact_pointer_pos().unwrap_or(grip.rect.center());
            // drag_started can share the frame that already moved the pointer,
            // so the press point is the current point minus this frame's delta.
            let origin = pointer - grip.drag_delta();
            let bar_min = ui
                .ctx()
                .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
                .map(|bar| bar.min)
                .unwrap_or(grip.rect.min);
            // A grab mid-slide continues from the pose already on screen.
            self.dock_drag_edge = Some(self.dock_layout_edge());
            self.dock_grab = origin - bar_min;
            self.dock_held = true;
            self.dock_moved = false;
            self.dock_vel = Vec2::ZERO;
            self.dock_slide = None;
            // The gesture starts on this frame, after steering. Place now so
            // the same move that crosses the drag threshold is kept.
            self.place_carried_dock(ui.ctx(), pointer);
        }
    }

    /// Follow the pointer while the case is carried. Runs before the case is
    /// painted, so a turn is on screen the same frame the side takes it.
    fn steer_carried_dock(&mut self, ctx: &Context) {
        if !self.dock_held {
            return;
        }
        let down = ctx.input(|i| i.pointer.any_down());
        let pos = ctx.input(|i| i.pointer.latest_pos());
        if down {
            if let Some(p) = pos {
                self.place_carried_dock(ctx, p);
            }
            return;
        }
        self.drop_carried_dock(ctx, pos);
    }

    fn place_carried_dock(&mut self, ctx: &Context, p: Pos2) {
        let delta = ctx.input(|i| i.pointer.delta());
        if delta.length() > 0.3 {
            self.dock_moved = true;
            let dt = ctx.input(|i| i.stable_dt).clamp(1.0 / 240.0, 0.05);
            self.dock_vel = clamp_speed(delta / dt, DOCK_VEL_MAX);
        } else {
            self.dock_vel *= 0.5;
        }
        let screen = ctx.screen_rect();
        let bounds = note_controls_bounds(ctx);
        let layout = self.dock_layout_edge();
        let mut size = self.dock_size.unwrap_or(vec2(48.0, 48.0));
        let hand = p - self.dock_grab;
        // Unclamped pose. The turn looks at this, not the rect already shoved
        // into the frame by the screen clamp.
        let body = Rect::from_min_size(hand, size);
        let edge = carrying_edge(p, screen, bounds, layout, body, self.dock_vel);
        if edge.vertical() != layout.vertical() {
            size = vec2(size.y, size.x).min(bounds.size());
            self.dock_size = Some(size);
        }
        self.dock_drag_edge = Some(edge);
        let rest = dock_rest_min(edge, self.dock_pill_size(edge.vertical()), bounds);
        let pull = dock_magnet(edge_distance(p, screen, edge));
        self.dock_float = Some(magnet_min(hand, rest, edge, pull));
    }

    fn drop_carried_dock(&mut self, ctx: &Context, pos: Option<Pos2>) {
        self.dock_held = false;
        let screen = ctx.screen_rect();
        let next = if self.dock_moved {
            snap_dock(pos.unwrap_or(screen.center()), screen)
        } else {
            self.dock_edge
        };
        let visual = self.dock_layout_edge();
        let old_pill = self.dock_pill_size(visual.vertical());
        let new_pill = self.dock_pill_size(next.vertical());
        let from = self.dock_float.unwrap_or(screen.center());
        if next.vertical() != visual.vertical() {
            if let Some(size) = self.dock_size {
                self.dock_size = Some(vec2(size.y, size.x));
            }
        }
        self.dock_edge = next;
        if self.lib.index.dock != next {
            self.lib.index.dock = next;
            let saved = self.lib.save_index();
            self.report_save(saved);
        }
        self.dock_vel = Vec2::ZERO;
        self.dock_moved = false;
        self.dock_drag_edge = Some(next);
        self.dock_float = Some(from + (old_pill - new_pill) * 0.5);
        self.dock_slide = Some(DockSlide {
            vel: Vec2::ZERO,
            shown: false,
            arrived: false,
        });
        ctx.request_repaint();
    }

    fn dock_inner(&mut self, ui: &mut Ui, _vertical: bool) {
        let slot = self.slot();
        for t in [
            Tool::Fineliner,
            Tool::Brush,
            Tool::Pencil,
            Tool::Highlighter,
        ] {
            if self.parked_tools.contains(&t) {
                continue;
            }
            if self.tool_glyph(ui, t) {
                self.finish_text_edit();
                if self.tool == t {
                    self.set_ink_panel(!self.ink_panel_open());
                } else {
                    self.sel.clear();
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
        let kind = self.last_eraser;
        let on = self.tool == kind || pen.eraser;
        if !self.parked_tools.contains(&Tool::EraserStroke) {
            let response = self.paint_tool_well(ui, kind, on);
            let (click, _) = Self::tool_button_action(&response);
            if click {
                self.finish_text_edit();
                self.sel.clear();
                self.pick_eraser(kind);
            }
        }
        if !self.parked_tools.contains(&Tool::Hand) {
            let hand = self.paint_tool_well(ui, Tool::Hand, self.tool == Tool::Hand);
            if hand.clicked() {
                self.finish_text_edit();
                self.tool = Tool::Hand;
            }
        }
        if !self.parked_tools.contains(&Tool::Text) && self.tool_glyph(ui, Tool::Text) {
            self.sel.clear();
            self.tool = Tool::Text;
        }
        if !self.parked_tools.contains(&Tool::Image) {
            let image_resp = self.paint_tool_well(ui, Tool::Image, self.tool == Tool::Image);
            if image_resp.clicked() {
                self.finish_text_edit();
                self.tool = Tool::Image;
                self.pending_image = Some(self.drop_spot());
            }
        }
        if self.ink_well(ui, self.ink_panel_open()).clicked() {
            self.set_ink_panel(!self.ink_panel_open());
        }
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
    }

    fn dock_grip(&self, ui: &mut Ui, vertical: bool) -> Response {
        let s = self.slot();
        let (rect, resp) = ui.allocate_exact_size(vec2(s, s), Sense::click_and_drag());
        resp.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Move toolbar")
        });
        let p = ui.painter();
        let c = rect.center();
        let opacity = if resp.hovered() || resp.dragged() || self.dock_held || self.dock_fold > 0.2
        {
            0.9
        } else {
            0.58
        };
        let color = self.look.paper.gamma_multiply(opacity);
        if vertical {
            for i in 0..3 {
                let x = c.x - 7.0 + i as f32 * 7.0;
                p.circle_filled(pos2(x, c.y - 3.5), 1.7, color);
                p.circle_filled(pos2(x, c.y + 3.5), 1.7, color);
            }
        } else {
            for i in 0..3 {
                let y = c.y - 7.0 + i as f32 * 7.0;
                p.circle_filled(pos2(c.x - 3.5, y), 1.7, color);
                p.circle_filled(pos2(c.x + 3.5, y), 1.7, color);
            }
        }
        let cursor = if resp.dragged() {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        };
        resp.on_hover_cursor(cursor)
    }

    /// Current ink at the current nib size. One well opens colour and thickness.
    fn ink_well(&self, ui: &mut Ui, open: bool) -> Response {
        let s = self.slot();
        let (rect, resp) = ui.allocate_exact_size(vec2(s, s), Sense::click());
        let c = rect.center();
        let t = nib_t(self.width);
        let r = 2.6 + t * 6.2;
        let p = ui.painter();
        p.circle_filled(c, r, self.ink_color());
        if open {
            p.circle_stroke(c, (r + 3.6).min(s * 0.42), Stroke::new(1.5, self.look.fg));
        }
        resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), open, "Ink"));
        resp.on_hover_cursor(CursorIcon::PointingHand)
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

    fn tool_glyph(&mut self, ui: &mut Ui, tool: Tool) -> bool {
        let response = self.paint_tool_well(ui, tool, self.tool == tool);
        let (click, options) = if tool.is_ink() {
            Self::tool_button_action(&response)
        } else {
            (response.clicked(), false)
        };
        if options {
            self.finish_text_edit();
            self.sel.clear();
            if self.tool != tool {
                self.width = default_width(tool.nib().unwrap());
            }
            self.tool = tool;
            self.last_ink = tool;
            self.last_ink_width = self.width;
            self.set_ink_panel(true);
            false
        } else {
            click
        }
    }

    fn paint_tool_well(&mut self, ui: &mut Ui, tool: Tool, active: bool) -> Response {
        let resp = self.round_well_with_sense(
            ui,
            self.slot(),
            self.tool_well_fill(tool, active),
            Sense::click_and_drag(),
            |p, c, fg| {
                paint_tool_icon(p, c, fg, tool);
            },
        );
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new(("tool-well", dock_tool_key(tool))), resp.rect));
        if resp.drag_started() {
            self.dock_tool_drag = Some(dock_tool_key(tool));
            self.set_ink_panel(true);
            self.color_mixer_open = false;
        }
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

        if !self.pdf_page_picker_open {
            if self.sheet_tabs_on {
                self.arm_page_marks(ui);
            }
            let touch_owned = self.handle_canvas_touch(ui, rect);
            self.handle_camera(ui, &resp, rect);
            let pen = self.tablet.snapshot();
            let leave_pages = self.sheet_tabs_on
                && ui.input(|i| i.pointer.button_double_clicked(PointerButton::Primary))
                && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| {
                    rect.contains(p)
                        && !Self::pen_over_chrome(ui.ctx(), p)
                        && self.sheet_tab_at(p, rect).is_none()
                        && self.page_minus_at(p, rect).is_none()
                        && self.sticky_minus_at(p, rect).is_none()
                        && self.image_minus_at(p, rect).is_none()
                })
                && self.page_drag.is_none();
            if leave_pages {
                self.leave_pages_mode();
            } else if self.sheet_tabs_on {
                if !touch_owned || pen.down || pen.pressed || pen.released {
                    self.pages_or_pan(ui, &resp, rect);
                }
            } else if !touch_owned || pen.down || pen.pressed || pen.released {
                self.handle_tool(ui, &resp, rect);
            }
        }
        self.paint_world(&painter, rect);
        self.paint_sheet_tabs(ui, &painter, rect);
        if self.sheet_tabs_on {
            self.paint_page_minuses(ui, &painter, rect);
            self.paint_sticky_minuses(ui, &painter, rect);
            self.paint_image_minuses(ui, &painter, rect);
            self.paint_page_drag(ui, &painter, rect);
            self.cursor_for_tool(ui, &resp);
            return;
        }
        self.paint_overlays(ui, rect);
        self.paint_area_eraser_lens(ui, &painter, rect);
        self.cursor_for_tool(ui, &resp);
        self.paint_selection_handles(ui, &painter, rect);
        self.paint_touch_hold(ui, &painter);
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
            let page = page.min(last);
            let origin = self.origin_of(page);
            if self.note_has_pdf() {
                // Cover-fit page 0 and keep it fitted while the dock settles.
                self.camera.fit_page(rect, origin, pw, ph);
                self.fitted_cell = self.page_cells().get(page).copied();
            } else {
                self.camera.show_writing(rect, origin, pw, ph);
                self.fitted_cell = None;
            }
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
        if self.selection_resize.is_some() || self.drag_last.is_some() {
            return;
        }
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
            // Trackpad pinch: zoom from finger distance; pan is already damped
            // in the tablet bridge while zoom is live (common-mode translation).
            if (pen.pinch_zoom - 1.0).abs() > 0.0005 {
                self.camera.zoom_at(focus, rect, pen.pinch_zoom);
            }
            let pan_gain = if (pen.pinch_zoom - 1.0).abs() > 0.0005 {
                1.0
            } else {
                1.65
            };
            self.camera.pan += pen.pinch_pan * pan_gain;
        } else if (egui_zoom - 1.0).abs() > 0.0005 {
            // Touchscreen pinch (and any remaining egui zoom).
            self.camera.zoom_at(focus, rect, egui_zoom);
            self.camera.pan += mt_pan;
        } else if ctrl && point_scroll != Vec2::ZERO {
            // Fallback when the pad ignores a resting finger (no pinch events):
            // ctrl + two-finger scroll zooms.
            let f = (1.0 + point_scroll.y * 0.006).clamp(0.7, 1.35);
            self.camera.zoom_at(focus, rect, f);
        } else if !ctrl && point_scroll != Vec2::ZERO {
            // Trackpad: two-finger scroll pans on both axes.
            self.camera.pan += point_scroll * 1.65;
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

        if self.camera.pan != before.pan
            || self.camera.zoom != before.zoom
            || self.camera.stretch_y != before.stretch_y
        {
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
            || {
                let pen = self.tablet.snapshot();
                pen.pinching
            }
        {
            self.drag_last = None;
            self.selection_resize = None;
            self.lasso.clear();
            return;
        }
        let pen = self.tablet.snapshot();
        let pen_ink =
            pen.down || pen.pressed || pen.released || (self.live.is_some() && self.live_from_pen);
        if !pen_ink && self.live.is_none() && self.palm_guard(&pen) {
            return;
        }

        let extra: Vec<Pos2>;
        let (mut screen, primary_down, mut primary_pressed, mut primary_released, secondary) =
            if pen_ink {
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
                let pos = resp
                    .interact_pointer_pos()
                    .or_else(|| resp.hover_pos())
                    .or_else(|| ui.input(|i| i.pointer.interact_pos()));
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

        if !rect.contains(screen)
            && self.drag_last.is_none()
            && self.selection_resize.is_none()
            && !(self.live.is_some() && (primary_released || !primary_down))
        {
            return;
        }
        if ui.ctx().data(|d| {
            d.get_temp::<Rect>(Id::new("selection-actions"))
                .is_some_and(|r| r.contains(screen))
                || d.get_temp::<Rect>(Id::new("dock-rect"))
                    .map(|r| r.expand(6.0).contains(screen))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("top-rect"))
                    .map(|r| r.expand(2.0).contains(screen))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("tin-rect"))
                    .map(|r| r.expand(2.0).contains(screen))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("nib-rect"))
                    .map(|r| r.expand(2.0).contains(screen))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("coach-button-rect"))
                    .map(|r| r.expand(4.0).contains(screen))
                    .unwrap_or(false)
        }) {
            if self.live.is_some() && (primary_released || !primary_down) {
                self.finish_live();
            }
            if primary_released || !primary_down {
                self.drag_last = None;
                self.selection_resize = None;
            }
            return;
        }
        if primary_pressed && self.tool != Tool::Lasso {
            if let Some((col, row)) = self.sheet_tab_at(screen, rect) {
                self.add_sheet_from_tab(col, row);
                self.tab_held = true;
                return;
            }
        }
        if self.take_pages_manage_hold(
            ui,
            resp,
            rect,
            &mut screen,
            pen_ink && pen.pressed,
            primary_down,
            &mut primary_pressed,
            &mut primary_released,
        ) {
            return;
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
                self.sel.clear();
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
            let r = self.eraser_radius_paper(self.camera.zoom, tool == Tool::EraserArea);
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

        if tool == Tool::Hand {
            if primary_pressed {
                self.selection_resize = None;
                if let Some((_, anchor)) = self.selection_corner(screen, rect) {
                    self.push_snapshot();
                    self.selection_resize = Some((anchor, paper - anchor, 1.0));
                    self.drag_last = None;
                    self.lasso.clear();
                } else if self.selection_bounds().is_some_and(|r| r.contains(paper)) {
                    self.push_snapshot();
                    self.drag_last = Some(paper);
                    self.lasso.clear();
                } else if let Some(hit) = self.hit_object(paper) {
                    self.sel = vec![hit];
                    self.finish_text_edit();
                    self.push_snapshot();
                    self.drag_last = Some(paper);
                    self.lasso.clear();
                } else {
                    self.sel.clear();
                    self.drag_last = None;
                }
            } else if primary_down {
                if let Some((anchor, initial, previous)) = self.selection_resize {
                    let scale = ((paper - anchor).dot(initial) / initial.length_sq().max(1.0))
                        .clamp(0.1, 10.0);
                    if (scale - previous).abs() > 0.0001 {
                        self.scale_selection(anchor, scale / previous);
                        self.selection_resize = Some((anchor, initial, scale));
                        self.mark_dirty();
                    }
                } else if let Some(last) = self.drag_last {
                    let d = paper - last;
                    self.translate_sel(d);
                    self.drag_last = Some(paper);
                    self.mark_dirty();
                }
            } else if primary_released {
                self.drag_last = None;
                self.selection_resize = None;
            }
        } else if tool == Tool::Lasso {
            if primary_pressed {
                self.selection_resize = None;
                self.drag_last = None;
                self.lasso.clear();
                self.sel.clear();
                self.lasso.push(paper);
                // A canceled page hold replays its origin. Keep the move
                // that canceled it too, otherwise the first corner is lost.
                for point in &extra {
                    let point = self.camera.to_paper(*point, rect);
                    if self
                        .lasso
                        .last()
                        .is_some_and(|last| last.distance(point) > 2.0)
                    {
                        self.lasso.push(point);
                    }
                }
            } else if primary_down {
                if self
                    .lasso
                    .last()
                    .map(|p| p.distance(paper) > 2.0)
                    .unwrap_or(true)
                {
                    self.lasso.push(paper);
                }
            } else if primary_released {
                if self.lasso.len() > 2 {
                    self.sel = self.hits_lasso();
                }
                self.lasso.clear();
            }
        } else if !primary_down {
            self.drag_last = None;
            self.selection_resize = None;
        }

        let tap = primary_pressed || resp.clicked();
        if tool == Tool::Text && tap {
            self.activate_text(paper);
        }

        if tool == Tool::Image && tap && self.image_import.is_none() {
            self.pending_image = Some((page, local));
        }
    }

    fn activate_text(&mut self, paper: Pos2) {
        let page = self.page_at_paper(paper);
        let local = paper - self.origin_of(page);
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
        let still_r = 14.0 / self.camera.zoom_min().max(0.15);
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
                let sticky = s.sticky;
                s = snapped;
                s.sticky = sticky;
            }
            if !s.points.is_empty() {
                if s.nib == Nib::Highlighter {
                    let extra = self.line_guides(p);
                    let y =
                        s.points.iter().map(|pt| pt.y).sum::<f32>() / s.points.len().max(1) as f32;
                    let page_h = self.ph();
                    let kind = self.note.as_ref().map(|n| n.paper).unwrap_or_default();
                    s.cling_to_line(snap_rule_y(kind, page_h, y, &extra));
                }
                if let Some(n) = &mut self.note {
                    if let Some(page) = n.pages.get_mut(p) {
                        page.strokes.push(s);
                    }
                }
                self.touch_document();
                self.coach_ink_changed(p);
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

    fn selection_bounds(&self) -> Option<Rect> {
        let n = self.note.as_ref()?;
        let mut bounds = Rect::NOTHING;
        for s in &self.sel {
            let Some(page) = n.pages.get(s.page) else {
                continue;
            };
            let o = self.origin_of(s.page);
            let hit = match s.kind {
                SelKind::Stroke => page.strokes.iter().find(|x| x.id == s.id).and_then(|st| {
                    st.bbox()
                        .map(|(a, b)| Rect::from_min_max(a + o, b + o).expand(st.width * 0.5))
                }),
                SelKind::Text => page
                    .texts
                    .iter()
                    .find(|x| x.id == s.id)
                    .map(|t| t.rect().translate(o)),
                SelKind::Image => page
                    .images
                    .iter()
                    .find(|x| x.id == s.id)
                    .map(|t| t.rect().translate(o)),
            };
            if let Some(hit) = hit {
                bounds = bounds.union(hit);
            }
        }
        bounds.is_finite().then_some(bounds)
    }

    fn hit_object(&self, paper: Pos2) -> Option<Sel> {
        let note = self.note.as_ref()?;
        for (page_index, page) in note.pages.iter().enumerate().rev() {
            let local = paper - self.origin_of(page_index);
            let hit = page
                .texts
                .iter()
                .rev()
                .find(|t| t.rect().contains(local))
                .map(|t| (t.id, SelKind::Text))
                .or_else(|| {
                    page.strokes
                        .iter()
                        .rev()
                        .find(|s| s.hits(local, 8.0 / self.camera.zoom_min()))
                        .map(|s| (s.id, SelKind::Stroke))
                })
                .or_else(|| {
                    page.images
                        .iter()
                        .rev()
                        .find(|im| im.rect().contains(local))
                        .map(|im| (im.id, SelKind::Image))
                });
            if let Some((id, kind)) = hit {
                return Some(Sel {
                    page: page_index,
                    id,
                    kind,
                });
            }
        }
        None
    }

    fn selection_corner(&self, screen: Pos2, canvas: Rect) -> Option<(Pos2, Pos2)> {
        let b = self.selection_bounds()?;
        [
            b.left_top(),
            b.right_top(),
            b.right_bottom(),
            b.left_bottom(),
        ]
        .into_iter()
        .filter_map(|corner| {
            let distance = self.camera.to_screen(corner, canvas).distance(screen);
            (distance <= 22.0).then_some((distance, corner))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, corner)| (corner, b.center() + (b.center() - corner)))
    }

    fn scale_selection(&mut self, anchor: Pos2, factor: f32) {
        let origins: Vec<_> = self.sel.iter().map(|s| self.origin_of(s.page)).collect();
        let Some(note) = &mut self.note else { return };
        for (s, origin) in self.sel.iter().zip(origins) {
            let Some(page) = note.pages.get_mut(s.page) else {
                continue;
            };
            let map = |p: Pos2| anchor + (p + origin - anchor) * factor - origin;
            match s.kind {
                SelKind::Stroke => {
                    if let Some(stroke) = page.strokes.iter_mut().find(|st| st.id == s.id) {
                        for point in &mut stroke.points {
                            let p = map(point.pos());
                            point.x = p.x;
                            point.y = p.y;
                        }
                        stroke.width *= factor;
                        stroke.mesh = None;
                    }
                }
                SelKind::Text => {
                    if let Some(text) = page.texts.iter_mut().find(|t| t.id == s.id) {
                        let p = map(text.min());
                        text.pos = [p.x, p.y];
                        text.size = [text.size[0] * factor, text.size[1] * factor];
                        text.size_pt *= factor;
                    }
                }
                SelKind::Image => {
                    if let Some(image) = page.images.iter_mut().find(|im| im.id == s.id) {
                        let p = map(image.min());
                        image.pos = [p.x, p.y];
                        image.size = [image.size[0] * factor, image.size[1] * factor];
                    }
                }
            }
        }
    }

    fn paint_selection_handles(&self, ui: &Ui, painter: &Painter, canvas: Rect) {
        if self.editing_text.is_some() {
            return;
        }
        let Some(b) = self.selection_bounds() else {
            return;
        };
        let b = Rect::from_min_max(
            self.camera.to_screen(b.min, canvas),
            self.camera.to_screen(b.max, canvas),
        );
        painter.rect_stroke(
            b,
            2,
            Stroke::new(1.2, self.look.accent),
            StrokeKind::Outside,
        );
        for corner in [
            b.left_top(),
            b.right_top(),
            b.right_bottom(),
            b.left_bottom(),
        ] {
            painter.circle_filled(corner + vec2(0.0, 1.5), 8.0, self.look.shadow);
            painter.circle_filled(corner, 8.0, self.look.paper);
            painter.circle_stroke(corner, 8.0, Stroke::new(1.5, self.look.accent));
        }
        if self.tool == Tool::Hand {
            if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
                if !Self::pen_over_chrome(ui.ctx(), p) && canvas.contains(p) {
                    if let Some((corner, anchor)) = self.selection_corner(p, canvas) {
                        ui.ctx().set_cursor_icon(
                            if (corner.x - anchor.x) * (corner.y - anchor.y) >= 0.0 {
                                CursorIcon::ResizeNwSe
                            } else {
                                CursorIcon::ResizeNeSw
                            },
                        );
                    } else if b.contains(p) {
                        ui.ctx().set_cursor_icon(if self.drag_last.is_some() {
                            CursorIcon::Grabbing
                        } else {
                            CursorIcon::Grab
                        });
                    }
                }
            }
        }
    }

    fn ui_selection_actions(&mut self, ctx: &Context) {
        ctx.data_mut(|d| d.remove::<Rect>(Id::new("selection-actions")));
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
                            let sticky = page
                                .texts
                                .iter()
                                .find(|x| x.id == s.id)
                                .is_some_and(|t| t.sticky);
                            if let Some(t) = page.texts.iter_mut().find(|x| x.id == s.id) {
                                t.translate(d);
                            }
                            if sticky {
                                for st in page.strokes.iter_mut().filter(|x| x.sticky == Some(s.id))
                                {
                                    st.translate(d);
                                }
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

    fn pick_image(&mut self, ctx: &Context, page: usize, local: Pos2, replace: Option<Uuid>) {
        if self.image_import.is_some() {
            return;
        }
        let Some(note) = &self.note else { return };
        let Some(target) = note.pages.get(page) else {
            return;
        };
        let (note, cell) = (note.id, (target.col, target.row));
        let task = rfd::AsyncFileDialog::new()
            .set_title("Place an image")
            .add_filter(
                "Images",
                &["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg"],
            )
            .pick_file();
        let (send, result) = mpsc::channel();
        let repaint = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("inkarchy-image-picker".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let Some(file) = pollster::block_on(task) else {
                        return ImageImportResult::Cancelled;
                    };
                    let path = file.path().to_path_buf();
                    if crate::import::is_placeable_image(&path) {
                        return crate::import::load_placed_image(&path)
                            .map(ImageImportResult::Ready)
                            .unwrap_or(ImageImportResult::Failed);
                    }
                    let Ok(bytes) = std::fs::read(&path) else {
                        return ImageImportResult::Failed;
                    };
                    ensure_png(&bytes)
                        .map(ImageImportResult::Ready)
                        .unwrap_or(ImageImportResult::Failed)
                }))
                .unwrap_or(ImageImportResult::Failed);
                let _ = send.send(outcome);
                repaint.request_repaint();
            });
        if spawned.is_ok() {
            self.image_import = Some(ImageImport {
                note,
                cell,
                local,
                replace,
                result,
            });
        } else {
            self.toast("Could not open image picker", ctx.input(|i| i.time));
        }
    }

    fn poll_image_import(&mut self, ctx: &Context) {
        let Some(job) = &self.image_import else {
            return;
        };
        let outcome = match job.result.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(ImageImportResult::Failed),
        };
        let Some(outcome) = outcome else {
            return;
        };
        let job = self.image_import.take().expect("image import was present");
        let Some(page) = self
            .note
            .as_ref()
            .filter(|n| n.id == job.note)
            .and_then(|n| n.pages.iter().position(|p| (p.col, p.row) == job.cell))
        else {
            return;
        };
        match outcome {
            ImageImportResult::Ready(png) => self.insert_png(&png, page, job.local, job.replace),
            ImageImportResult::Cancelled => {}
            ImageImportResult::Failed => self.toast("Could not load image", ctx.input(|i| i.time)),
        }
    }

    fn insert_image_bytes(&mut self, bytes: &[u8], page: usize, local: Pos2) {
        let Some(png) = ensure_png(bytes) else {
            return;
        };
        self.insert_png(&png, page, local, None);
    }

    fn insert_png(&mut self, png: &[u8], page: usize, local: Pos2, replace: Option<Uuid>) {
        let Some(note) = &self.note else {
            return;
        };
        let Some(target) = note.pages.get(page) else {
            return;
        };
        if replace.is_some_and(|id| !target.images.iter().any(|im| im.id == id)) {
            return;
        }
        let id = note.id;
        let Some(file) = self.lib.write_media(id, png) else {
            return;
        };
        let path = self.lib.media_path(id, &file);
        let (w, h) = image_size(&path).unwrap_or((400.0, 300.0));
        let scale = (420.0 / w.max(h)).min(1.0);
        self.push_snapshot();
        let image_id = replace.unwrap_or_else(Uuid::new_v4);
        if let Some(n) = &mut self.note {
            if let Some(pg) = n.pages.get_mut(page) {
                if let Some(image) = pg.images.iter_mut().find(|im| im.id == image_id) {
                    let scale = (image.size[0] / w).min(image.size[1] / h);
                    image.size = [w * scale, h * scale];
                    image.file = file;
                } else {
                    pg.images.push(ImageObj {
                        id: image_id,
                        pos: [local.x, local.y],
                        size: [w * scale, h * scale],
                        file,
                    });
                }
            }
        }
        self.finish_text_edit();
        self.tool = Tool::Hand;
        self.sel = vec![Sel {
            page,
            id: image_id,
            kind: SelKind::Image,
        }];
        self.mark_dirty();
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
                        std::env::temp_dir().join(format!("inkarchy-drop-{}.pdf", Uuid::new_v4()));
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
                    let (page, local) = ctx
                        .pointer_latest_pos()
                        .filter(|p| self.canvas_rect.contains(*p))
                        .map(|p| {
                            let paper = self.camera.to_paper(p, self.canvas_rect);
                            let page = self.page_at_paper(paper);
                            (page, paper - self.origin_of(page))
                        })
                        .unwrap_or_else(|| self.drop_spot());
                    let local = local + vec2(i as f32 * 24.0, i as f32 * 24.0);
                    self.insert_image_bytes(bytes, page, local);
                }
            }
        }
        let paste_img = !ctx.wants_keyboard_input()
            && self.editing_text.is_none()
            && ctx.input(|i| i.modifiers.command && i.key_pressed(Key::V));
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
                            let (page, local) = self.drop_spot();
                            self.insert_image_bytes(&png, page, local);
                        }
                    }
                }
            }
        }
    }

    fn paint_world(&mut self, painter: &Painter, rect: Rect) {
        self.bg_hot.clear();
        let Some(note) = self.note.as_ref().map(Note::clone_for_paint) else {
            return;
        };
        let surface = self.look.paper_surface(note.paper_tone);
        let canson = if note.paper_tone == PaperTone::Black {
            None
        } else {
            Some(self.paper_tex(painter.ctx()))
        };
        let cam = self.camera;
        let map = |p: Pos2| cam.to_screen(p, rect);
        let now = painter.ctx().input(|i| i.time);
        let fused =
            note.sheet_join == SheetJoin::Linked && note.pages.len() > 1 && !note.is_grown_single();
        let min_col = note.pages.iter().map(|p| p.col).min().unwrap_or(0);
        let fill = surface;
        let bleed = 1.6;
        let occupied: HashSet<_> = note.pages.iter().map(|p| (p.col, p.row)).collect();
        let pixels_per_point = painter.ctx().pixels_per_point();
        let view_scale = [cam.zoom, cam.zoom_y(), pixels_per_point];
        if self.pdf_view_scale != view_scale {
            self.pdf_view_scale = view_scale;
            self.pdf_detail_after = now + 0.12;
        }
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
                let jiggle = self.page_jiggle_offset(page.col, page.row, now);
                let paper = Rect::from_min_max(min, max).translate(jiggle);
                let n_left = fused && occupied.contains(&(page.col - 1, page.row));
                let n_right = fused && occupied.contains(&(page.col + 1, page.row));
                let n_top = fused && occupied.contains(&(page.col, page.row - 1));
                let n_bot = fused && occupied.contains(&(page.col, page.row + 1));
                (
                    pi, page, origin, paper, n_left, n_right, n_top, n_bot, jiggle,
                )
            })
            .collect();

        // Finish every opaque sheet before drawing grain, ruling or user content.
        // A later page must never paint over its neighbour's grid or ink.
        let mut surfaces = Vec::with_capacity(tiles.len());
        for &(_, page, _, paper, n_left, n_right, n_top, n_bot, _) in &tiles {
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
            if !fused {
                self.paint_template(&surface_painter, paper, note.paper, cam.scale());
            }
        }
        if fused {
            self.paint_fused_rules(painter, &note, cam, rect);
        }

        for &(pi, page, origin, paper, _, _, _, _, jiggle) in &tiles {
            let has_bg = page.background.is_some();
            if !fused && !has_bg && note.paper.has_margin_punches() && page.col == min_col {
                self.paint_punches(painter, paper, cam.zoom_min(), note.paper_tone);
            }
            if has_bg && paper.expand(bleed).intersects(rect) {
                if let Some(bg) = page.background.as_ref() {
                    let [x, y, w, h] = bg.fit(note.page_w, note.page_h);
                    let r = Rect::from_min_size(
                        map(pos2(origin.x + x, origin.y + y)) + jiggle,
                        vec2(w * cam.zoom, h * cam.zoom_y()),
                    );
                    if let Some(tex) =
                        self.background_tex(painter.ctx(), note.id, bg, r, pixels_per_point)
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
            for tx in &page.texts {
                if !tx.sticky {
                    continue;
                }
                let r = Rect::from_min_size(
                    map(tx.min() + origin) + jiggle,
                    vec2(tx.size[0] * cam.zoom, tx.size[1] * cam.zoom_y()),
                );
                paint_sticky_card(painter, r, tx.color32(), self.look.shadow);
            }

            for im in &page.images {
                let r = Rect::from_min_size(
                    map(im.min() + origin) + jiggle,
                    vec2(im.size[0] * cam.zoom, im.size[1] * cam.zoom_y()),
                );
                if let Some(tex) = self.texture_for(painter.ctx(), &note.id, &im.file) {
                    painter.image(
                        tex.id(),
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    painter.rect_filled(r, 0.0, self.look.paper_rule(note.paper_tone));
                }
            }

            let sx = cam.zoom;
            let sy = cam.zoom_y();
            let ox = rect.min.x + cam.pan.x + origin.x * sx + jiggle.x;
            let oy = rect.min.y + cam.pan.y + origin.y * sy + jiggle.y;
            if let Some(n) = &mut self.note {
                if let Some(pg) = n.pages.get_mut(pi) {
                    for pass in [false, true] {
                        for s in &mut pg.strokes {
                            if s.nib == Nib::Highlighter {
                                if pass {
                                    continue;
                                }
                            } else if !pass {
                                continue;
                            }
                            if !ink_visible(s, ox, oy, sx, sy, rect) {
                                continue;
                            }
                            painter.add(Shape::mesh(map_mesh(s.tessellate(), ox, oy, sx, sy)));
                        }
                    }
                }
            }

            for tx in &page.texts {
                if self.editing_text == Some((pi, tx.id)) {
                    if tx.text.is_empty() {
                        let pos = map(tx.min() + origin) + jiggle;
                        let h = tx.size_pt * cam.zoom_y();
                        painter.line_segment(
                            [pos + vec2(0.6, 1.0), pos + vec2(0.6, h)],
                            Stroke::new(1.7_f32, tx.color32()),
                        );
                    }
                    continue;
                }
                let pos = map(tx.min() + origin) + jiggle;
                let max_w = (tx.size[0] * cam.zoom).max(8.0);
                let font = self.look.serif(tx.size_pt * cam.zoom_y());
                let col = if tx.sticky {
                    self.look.ink
                } else if note.paper_tone == PaperTone::Black && tx.color32() == self.look.ink {
                    self.look.paper_ink(note.paper_tone)
                } else {
                    tx.color32()
                };
                let galley = painter.layout(tx.text.clone(), font, col, max_w);
                painter.galley(pos, galley, col);
            }

            if !fused && page.background.is_none() {
                painter.text(
                    pos2(
                        paper.center().x,
                        paper.max.y - 14.0 * cam.zoom_min().min(1.2),
                    ),
                    Align2::CENTER_BOTTOM,
                    format!("{}", pi + 1),
                    self.look.serif(12.0),
                    self.look.paper_ink(note.paper_tone).gamma_multiply(0.38),
                );
            }
        }

        if self.sheet_tabs_on {
            if let Some((col, row)) = self.pages_jiggle_cell {
                let selected = tiles
                    .iter()
                    .find(|(_, page, ..)| page.col == col && page.row == row)
                    .map(|(_, _, _, paper, ..)| *paper);
                if let Some(paper) = selected {
                    painter.rect_stroke(
                        paper.expand(2.0),
                        6.0,
                        Stroke::new(2.0, self.look.accent),
                        StrokeKind::Outside,
                    );
                }
            }
        }

        if let Some(pi) = self.live.as_ref().map(|(pi, _)| *pi) {
            let origin = self.origin_of(pi);
            let sx = cam.zoom;
            let sy = cam.zoom_y();
            let ox = rect.min.x + cam.pan.x + origin.x * sx;
            let oy = rect.min.y + cam.pan.y + origin.y * sy;
            let mesh = if let Some(preview) = self.shape_preview.as_mut() {
                Some(map_mesh(preview.tessellate(), ox, oy, sx, sy))
            } else {
                self.live
                    .as_mut()
                    .map(|(_, stroke)| map_mesh(stroke.tessellate(), ox, oy, sx, sy))
            };
            if let Some(mesh) = mesh {
                painter.add(Shape::mesh(mesh));
            }
        }
        if self.lasso.len() >= 2 {
            let pts: Vec<_> = self.lasso.iter().copied().map(map).collect();
            painter.add(Shape::closed_line(pts, Stroke::new(1.2, self.look.accent)));
        }
        if let Some(hub) = &self.pdf_raster {
            for key in hub.retain_visible(&self.bg_hot) {
                self.bg_pending.remove(&key);
            }
        }
    }

    fn tear_unit_at(&mut self, col: i32, row: i32) {
        if !self
            .note
            .as_ref()
            .is_some_and(|note| note.can_tear_unit() && note.unit_occupied(col, row))
        {
            return;
        }
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.sel.clear();
        self.lasso.clear();
        self.push_snapshot();
        if let Some(note) = &mut self.note {
            note.remove_unit(col, row);
        }
        if self.pages_jiggle_cell == Some((col, row)) {
            self.pages_jiggle_cell = None;
        }
        // Stay at the current zoom. A cover-fit here jumps the sheet to the window.
        self.fitted_cell = None;
        self.mark_dirty();
    }

    fn unit_screen_rect(&self, canvas: Rect, col: i32, row: i32) -> Rect {
        let pw = self.pw();
        let ph = self.ph();
        let o = page_origin(col, row, pw, ph, self.gap());
        let min = self.camera.to_screen(Pos2::new(o.x, o.y), canvas);
        let max = self.camera.to_screen(Pos2::new(o.x + pw, o.y + ph), canvas);
        Rect::from_min_max(min, max)
    }

    fn sheet_tabs(&self, canvas: Rect) -> Vec<(i32, i32, Rect, i32, i32, bool)> {
        if !self.sheet_tabs_on {
            return Vec::new();
        }
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
                Some((t.dest_col, t.dest_row, hit, t.src_col, t.src_row, t.center))
            })
            .collect()
    }

    fn sheet_tab_at(&self, screen: Pos2, canvas: Rect) -> Option<(i32, i32)> {
        if self.note.is_none() || canvas.width() < 10.0 {
            return None;
        }
        self.sheet_tabs(canvas)
            .into_iter()
            .filter(|(_, _, r, ..)| r.contains(screen))
            .min_by(|a, b| {
                let da = a.2.center().distance(screen);
                let db = b.2.center().distance(screen);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(c, r, ..)| (c, r))
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
        self.pages_jiggle_cell = Some((col, row));
        self.sel.clear();
        self.lasso.clear();
        self.mark_dirty();
    }

    fn paint_sheet_tabs(&self, ui: &Ui, painter: &Painter, canvas: Rect) {
        if self.note.is_none() || canvas.width() < 10.0 {
            return;
        }
        let (hover, time) = ui.input(|i| (i.pointer.hover_pos(), i.time));
        let (fill, fill_hot, glyph) = sheet_mark_colors(self.look.green(), self.look.paper);
        for (_, _, hit, src_col, src_row, center) in self.sheet_tabs(canvas) {
            if !hit.is_positive() {
                continue;
            }
            let painted = if center {
                hit
            } else {
                hit.translate(self.page_jiggle_offset(src_col, src_row, time))
            };
            let hot = hover.is_some_and(|p| hit.contains(p));
            paint_sheet_tab(
                painter,
                painted,
                if hot { fill_hot } else { fill },
                glyph,
                self.look.shadow,
                false,
            );
        }
    }

    fn toggle_pages_mode(&mut self) {
        if self.sheet_tabs_on {
            self.leave_pages_mode();
        } else {
            self.enter_pages_mode();
        }
    }

    fn clear_page_drag_state(&mut self) {
        self.pages_manage_hold = None;
        self.page_drag_hold = None;
        self.page_drag = None;
    }

    fn leave_pages_mode(&mut self) {
        self.tab_held = false;
        self.page_press_owned = false;
        self.page_mark = None;
        self.pages_jiggle_cell = None;
        self.pages_marks_ready = false;
        self.clear_page_drag_state();
        self.sheet_tabs_on = false;
    }

    fn enter_pages_mode(&mut self) {
        let target = self.fitted_cell.or_else(|| {
            self.note
                .as_ref()
                .and_then(|note| note.pages.first())
                .map(|page| (page.col, page.row))
        });
        self.enter_pages_mode_at(target);
    }

    fn enter_pages_mode_at(&mut self, target: Option<(i32, i32)>) {
        if self.sheet_tabs_on || self.note.is_none() {
            return;
        }
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.sel.clear();
        self.lasso.clear();
        self.reset_canvas_gesture();
        self.set_ink_panel(false);
        self.page_mark = None;
        self.clear_page_drag_state();
        self.page_press_owned = false;
        self.pages_marks_ready = false;
        self.pages_jiggle_cell = target;
        self.sheet_tabs_on = true;
    }

    fn start_page_drag(&mut self, col: i32, row: i32, screen: Pos2, canvas: Rect) {
        let paper = self.unit_screen_rect(canvas, col, row);
        self.page_drag = Some(PageDrag {
            col,
            row,
            grab: screen - paper.min,
            screen,
        });
        self.page_press_owned = true;
    }

    /// Kept as a single layout hook for page tabs and drag hit targets.
    fn page_jiggle_offset(&self, _col: i32, _row: i32, _time: f64) -> Vec2 {
        Vec2::ZERO
    }

    /// The click that opened Pages mode cannot land on a mark that just appeared.
    fn arm_page_marks(&mut self, ui: &Ui) {
        if !self.sheet_tabs_on || self.pages_marks_ready {
            return;
        }
        let pen = self.tablet.snapshot();
        let (down, pressed) = ui.input(|i| (i.pointer.primary_down(), i.pointer.primary_pressed()));
        if !down && !pressed && !pen.down && !pen.pressed {
            self.pages_marks_ready = true;
        }
    }

    fn pointer_at(&self, ui: &Ui) -> Option<Pos2> {
        let pen = self.tablet.snapshot();
        // A hovering stylus must not steal a finger or mouse tap.
        if pen.down || pen.pressed || pen.released {
            if let Some(pos) = pen.pos {
                return Some(pos);
            }
        }
        ui.input(|i| {
            let mut pos = i.pointer.interact_pos().or(i.pointer.hover_pos());
            for event in i.events.iter().rev() {
                match event {
                    Event::PointerMoved(p)
                    | Event::PointerButton { pos: p, .. }
                    | Event::Touch { pos: p, .. } => {
                        pos = Some(*p);
                        break;
                    }
                    _ => {}
                }
            }
            pos
        })
    }

    fn page_minus_at(&self, screen: Pos2, canvas: Rect) -> Option<(i32, i32)> {
        let note = self.note.as_ref()?;
        if !self.sheet_tabs_on || !note.can_tear_unit() || canvas.width() < 10.0 {
            return None;
        }
        let mut best: Option<(f32, i32, i32)> = None;
        for page in &note.pages {
            let paper = self.unit_screen_rect(canvas, page.col, page.row);
            let band = page_minus_band(paper);
            if !band.is_positive() || !band.contains(screen) {
                continue;
            }
            let distance = screen.distance(band.center());
            if best.is_none_or(|(best_d, _, _)| distance < best_d) {
                best = Some((distance, page.col, page.row));
            }
        }
        best.map(|(_, col, row)| (col, row))
    }

    fn paint_page_minuses(&self, ui: &Ui, painter: &Painter, canvas: Rect) {
        let Some(note) = self.note.as_ref() else {
            return;
        };
        if !note.can_tear_unit() || canvas.width() < 10.0 {
            return;
        }
        let (hover, time) = ui.input(|i| (i.pointer.hover_pos(), i.time));
        let (fill, fill_hot, glyph) = sheet_mark_colors(self.look.rust(), self.look.paper);
        for page in &note.pages {
            let paper = self.unit_screen_rect(canvas, page.col, page.row);
            let rest = page_minus_band(paper);
            let band = rest.translate(self.page_jiggle_offset(page.col, page.row, time));
            if !band.is_positive() {
                continue;
            }
            let hot = hover.is_some_and(|p| rest.contains(p));
            paint_sheet_tab(
                painter,
                band,
                if hot { fill_hot } else { fill },
                glyph,
                self.look.shadow,
                true,
            );
        }
    }

    fn commit_page_mark(&mut self, mark: &PageMarkPress) {
        match mark.kind {
            PageMark::Add(col, row) => self.add_sheet_from_tab(col, row),
            PageMark::Tear(col, row) => self.tear_unit_at(col, row),
            PageMark::TearSticky(page, id) => self.delete_sticky(page, id),
            PageMark::TearImage(page, id) => self.delete_image(page, id),
        }
    }

    fn delete_sticky(&mut self, page: usize, id: Uuid) {
        let present = self.note.as_ref().is_some_and(|note| {
            note.pages
                .get(page)
                .is_some_and(|p| p.texts.iter().any(|t| t.id == id && t.sticky))
        });
        if !present {
            return;
        }
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.sel.clear();
        self.lasso.clear();
        self.push_snapshot();
        if let Some(note) = &mut self.note {
            if let Some(pg) = note.pages.get_mut(page) {
                pg.texts.retain(|t| t.id != id);
                pg.strokes.retain(|s| s.sticky != Some(id));
            }
        }
        self.mark_dirty();
    }

    fn sticky_screen_rect(&self, canvas: Rect, page: usize, tx: &TextBox) -> Rect {
        let origin = self.origin_of(page);
        Rect::from_min_size(
            self.camera.to_screen(tx.min() + origin, canvas),
            vec2(
                tx.size[0] * self.camera.zoom,
                tx.size[1] * self.camera.zoom_y(),
            ),
        )
    }

    fn sticky_minus_at(&self, screen: Pos2, canvas: Rect) -> Option<(usize, Uuid)> {
        let note = self.note.as_ref()?;
        if !self.sheet_tabs_on || canvas.width() < 10.0 {
            return None;
        }
        let mut best: Option<(f32, usize, Uuid)> = None;
        for (page_index, page) in note.pages.iter().enumerate() {
            for tx in page.texts.iter().rev() {
                if !tx.sticky {
                    continue;
                }
                let band = sticky_minus_band(self.sticky_screen_rect(canvas, page_index, tx));
                if !band.is_positive() || !band.contains(screen) {
                    continue;
                }
                let distance = screen.distance(band.center());
                if best.is_none_or(|(best_d, _, _)| distance < best_d) {
                    best = Some((distance, page_index, tx.id));
                }
            }
        }
        best.map(|(_, page, id)| (page, id))
    }

    fn image_screen_rect(&self, canvas: Rect, page: usize, image: &ImageObj) -> Rect {
        let origin = self.origin_of(page);
        Rect::from_min_size(
            self.camera.to_screen(image.min() + origin, canvas),
            vec2(
                image.size[0] * self.camera.zoom,
                image.size[1] * self.camera.zoom_y(),
            ),
        )
    }

    fn image_minus_at(&self, screen: Pos2, canvas: Rect) -> Option<(usize, Uuid)> {
        let note = self.note.as_ref()?;
        if !self.sheet_tabs_on || canvas.width() < 10.0 {
            return None;
        }
        let mut best: Option<(f32, usize, Uuid)> = None;
        for (page_index, page) in note.pages.iter().enumerate().rev() {
            for image in page.images.iter().rev() {
                let band = image_minus_band(self.image_screen_rect(canvas, page_index, image));
                if !band.is_positive() || !band.contains(screen) {
                    continue;
                }
                let distance = screen.distance(band.center());
                if best.is_none_or(|(best_d, _, _)| distance < best_d) {
                    best = Some((distance, page_index, image.id));
                }
            }
        }
        best.map(|(_, page, id)| (page, id))
    }

    fn paint_image_minuses(&self, ui: &Ui, painter: &Painter, canvas: Rect) {
        let Some(note) = self.note.as_ref() else {
            return;
        };
        let hover = ui.input(|i| i.pointer.hover_pos());
        let (fill, fill_hot, glyph) = sheet_mark_colors(self.look.rust(), self.look.paper);
        for (page_index, page) in note.pages.iter().enumerate() {
            for image in &page.images {
                let band = image_minus_band(self.image_screen_rect(canvas, page_index, image));
                if !band.is_positive() || !band.intersects(canvas) {
                    continue;
                }
                let hot = hover.is_some_and(|p| band.contains(p));
                paint_sheet_tab(
                    painter,
                    band,
                    if hot { fill_hot } else { fill },
                    glyph,
                    self.look.shadow,
                    true,
                );
            }
        }
    }

    fn delete_image(&mut self, page: usize, id: Uuid) {
        let present = self.note.as_ref().is_some_and(|note| {
            note.pages
                .get(page)
                .is_some_and(|p| p.images.iter().any(|image| image.id == id))
        });
        if !present {
            return;
        }
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.sel.clear();
        self.lasso.clear();
        self.push_snapshot();
        if let Some(note) = &mut self.note {
            if let Some(page) = note.pages.get_mut(page) {
                page.images.retain(|image| image.id != id);
            }
        }
        self.mark_dirty();
    }

    fn paint_sticky_minuses(&self, ui: &Ui, painter: &Painter, canvas: Rect) {
        let Some(note) = self.note.as_ref() else {
            return;
        };
        if canvas.width() < 10.0 {
            return;
        }
        let (hover, time) = ui.input(|i| (i.pointer.hover_pos(), i.time));
        let (fill, fill_hot, glyph) = sheet_mark_colors(self.look.rust(), self.look.paper);
        for (page_index, page) in note.pages.iter().enumerate() {
            for tx in &page.texts {
                if !tx.sticky {
                    continue;
                }
                let rest = sticky_minus_band(self.sticky_screen_rect(canvas, page_index, tx));
                let band = rest.translate(self.page_jiggle_offset(page.col, page.row, time));
                if !band.is_positive() {
                    continue;
                }
                let hot = hover.is_some_and(|p| rest.contains(p));
                paint_sheet_tab(
                    painter,
                    band,
                    if hot { fill_hot } else { fill },
                    glyph,
                    self.look.shadow,
                    true,
                );
            }
        }
    }

    /// Page under the pointer in screen space (unit cell).
    fn page_unit_at(&self, screen: Pos2, canvas: Rect) -> Option<(i32, i32)> {
        let note = self.note.as_ref()?;
        if canvas.width() < 10.0 {
            return None;
        }
        let mut best: Option<(f32, i32, i32)> = None;
        for page in &note.pages {
            let paper = self.unit_screen_rect(canvas, page.col, page.row);
            if !paper.is_positive() || !paper.contains(screen) {
                continue;
            }
            let distance = screen.distance(paper.center());
            if best.is_none_or(|(best_d, _, _)| distance < best_d) {
                best = Some((distance, page.col, page.row));
            }
        }
        best.map(|(_, col, row)| (col, row))
    }

    /// Neighbour slots (+ destinations) and other pages — drop targets while dragging.
    fn page_drop_target(
        &self,
        screen: Pos2,
        canvas: Rect,
        from_col: i32,
        from_row: i32,
    ) -> Option<(i32, i32)> {
        let note = self.note.as_ref()?;
        if let Some((col, row)) = self.page_unit_at(screen, canvas) {
            if col != from_col || row != from_row {
                return Some((col, row));
            }
        }
        let mut best: Option<(f32, i32, i32)> = None;
        for tab in note.unit_tabs() {
            let hole = self.unit_screen_rect(canvas, tab.dest_col, tab.dest_row);
            if !hole.is_positive() || !hole.contains(screen) {
                continue;
            }
            let distance = screen.distance(hole.center());
            if best.is_none_or(|(best_d, _, _)| distance < best_d) {
                best = Some((distance, tab.dest_col, tab.dest_row));
            }
        }
        best.map(|(_, col, row)| (col, row))
    }

    fn move_page_unit(&mut self, from_col: i32, from_row: i32, to_col: i32, to_row: i32) {
        let ok = self.note.as_ref().is_some_and(|n| {
            n.cell_taken(from_col, from_row)
                && (from_col != to_col || from_row != to_row)
                && (n.cell_taken(to_col, to_row)
                    || n.unit_tabs()
                        .iter()
                        .any(|t| t.dest_col == to_col && t.dest_row == to_row))
        });
        if !ok {
            return;
        }
        self.push_snapshot();
        if let Some(note) = &mut self.note {
            note.move_unit(from_col, from_row, to_col, to_row);
        }
        if self.pages_jiggle_cell == Some((from_col, from_row)) {
            self.pages_jiggle_cell = Some((to_col, to_row));
        }
        self.fitted_cell = None;
        self.mark_dirty();
    }

    /// Tap `+` / `−`, long-press then drag a page into a neighbour slot, or pan.
    fn pages_or_pan(&mut self, ui: &Ui, resp: &Response, rect: Rect) {
        if self.tab_held {
            let down = ui.input(|i| i.pointer.primary_down()) || self.tablet.snapshot().down;
            self.tab_held = false;
            if down {
                self.tab_held = true;
            } else {
                self.page_press_owned = false;
            }
            return;
        }
        let pen = self.tablet.snapshot();
        let pos = self.pointer_at(ui);
        let chrome = pos.is_some_and(|p| Self::pen_over_chrome(ui.ctx(), p));
        let pressed = pen.pressed || ui.input(|i| i.pointer.primary_pressed());
        let released = pen.released || ui.input(|i| i.pointer.primary_released());
        let down = pen.down || ui.input(|i| i.pointer.primary_down());
        let time = ui.input(|i| i.time);
        let pan = |app: &mut Self| {
            if chrome {
                return;
            }
            let delta = resp.drag_delta();
            if delta != Vec2::ZERO {
                app.camera.pan += delta;
                app.fitted_cell = None;
            }
        };

        if let Some(mut drag) = self.page_drag.take() {
            if let Some(p) = pos {
                drag.screen = p;
            }
            if released || !down {
                if let Some(p) = pos {
                    if let Some((to_col, to_row)) =
                        self.page_drop_target(p, rect, drag.col, drag.row)
                    {
                        self.move_page_unit(drag.col, drag.row, to_col, to_row);
                    }
                }
                self.page_press_owned = false;
                return;
            }
            self.page_drag = Some(drag);
            self.page_press_owned = true;
            return;
        }

        if let Some(hold) = self.page_drag_hold.take() {
            if let Some(p) = pos {
                if p.distance(hold.screen) > 8.0 {
                    pan(self);
                    self.page_press_owned = true;
                    return;
                }
            }
            if time - hold.t0 >= WHEEL_HOLD {
                let paper = self.unit_screen_rect(rect, hold.col, hold.row);
                let screen = pos.unwrap_or(hold.screen);
                self.page_drag = Some(PageDrag {
                    col: hold.col,
                    row: hold.row,
                    grab: screen - paper.min,
                    screen,
                });
                self.page_press_owned = true;
                return;
            }
            if released || !down {
                self.page_press_owned = false;
                return;
            }
            ui.ctx().request_repaint_after(Duration::from_secs_f64(
                (WHEEL_HOLD - (time - hold.t0)).max(0.001),
            ));
            self.page_drag_hold = Some(hold);
            self.page_press_owned = true;
            return;
        }

        if let Some(mut mark) = self.page_mark.take() {
            if let Some(p) = pos {
                mark.travel = mark.travel.max(p.distance(mark.origin));
            }
            if released || !down {
                // A wobble still counts. Only a real drag gives up the mark.
                if self.pages_marks_ready && mark.travel <= PAGE_MARK_SLOP {
                    self.commit_page_mark(&mark);
                } else if mark.travel > PAGE_MARK_SLOP {
                    pan(self);
                }
                self.page_press_owned = false;
                return;
            }
            if mark.travel > PAGE_MARK_SLOP {
                pan(self);
                self.page_press_owned = true;
            } else {
                self.page_mark = Some(mark);
            }
            return;
        }

        if self.pages_marks_ready && pressed && pos.is_some_and(|p| rect.contains(p)) && !chrome {
            let p = pos.unwrap();
            if let Some((page, id)) = self.sticky_minus_at(p, rect) {
                if let Some(cell) = self
                    .note
                    .as_ref()
                    .and_then(|note| note.pages.get(page))
                    .map(|page| (page.col, page.row))
                {
                    self.pages_jiggle_cell = Some(cell);
                }
                self.page_mark = Some(PageMarkPress {
                    kind: PageMark::TearSticky(page, id),
                    origin: p,
                    travel: 0.0,
                });
                self.page_press_owned = true;
                return;
            }
            if let Some((page, id)) = self.image_minus_at(p, rect) {
                if let Some(cell) = self
                    .note
                    .as_ref()
                    .and_then(|note| note.pages.get(page))
                    .map(|page| (page.col, page.row))
                {
                    self.pages_jiggle_cell = Some(cell);
                }
                self.page_mark = Some(PageMarkPress {
                    kind: PageMark::TearImage(page, id),
                    origin: p,
                    travel: 0.0,
                });
                self.page_press_owned = true;
                return;
            }
            if let Some((col, row)) = self.page_minus_at(p, rect) {
                self.pages_jiggle_cell = Some((col, row));
                self.page_mark = Some(PageMarkPress {
                    kind: PageMark::Tear(col, row),
                    origin: p,
                    travel: 0.0,
                });
                self.page_press_owned = true;
                return;
            }
            if let Some((col, row)) = self.sheet_tab_at(p, rect) {
                self.page_mark = Some(PageMarkPress {
                    kind: PageMark::Add(col, row),
                    origin: p,
                    travel: 0.0,
                });
                self.page_press_owned = true;
                return;
            }
            if let Some((col, row)) = self.page_unit_at(p, rect) {
                self.pages_jiggle_cell = Some((col, row));
                self.page_drag_hold = Some(PageDragHold {
                    col,
                    row,
                    screen: p,
                    t0: time,
                });
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(WHEEL_HOLD));
                self.page_press_owned = true;
                return;
            }
        }
        if down && !chrome {
            pan(self);
        }
        if released || !down {
            self.page_press_owned = false;
        }
    }

    fn paint_page_drag(&self, ui: &Ui, painter: &Painter, canvas: Rect) {
        let Some(drag) = self.page_drag else {
            return;
        };
        let note = match self.note.as_ref() {
            Some(n) => n,
            None => return,
        };
        let (hover, time) = ui.input(|i| (i.pointer.hover_pos(), i.time));
        let hover = hover.or(Some(drag.screen));
        let accent = self.look.accent;
        let sheet_r = 5.0;
        for tab in note.unit_tabs() {
            let hole = self.unit_screen_rect(canvas, tab.dest_col, tab.dest_row);
            if !hole.is_positive() {
                continue;
            }
            let hot = hover.is_some_and(|p| hole.contains(p));
            let paper = self.look.paper;
            painter.rect_filled(
                hole,
                sheet_r,
                Color32::from_rgba_unmultiplied(
                    paper.r(),
                    paper.g(),
                    paper.b(),
                    if hot { 88 } else { 36 },
                ),
            );
            painter.rect_stroke(
                hole,
                sheet_r,
                Stroke::new(
                    if hot { 1.6 } else { 1.0 },
                    if hot {
                        self.look.ink.gamma_multiply(0.55)
                    } else {
                        Color32::from_rgba_unmultiplied(paper.r(), paper.g(), paper.b(), 170)
                    },
                ),
                StrokeKind::Inside,
            );
        }
        for page in &note.pages {
            if page.col == drag.col && page.row == drag.row {
                continue;
            }
            let paper = self
                .unit_screen_rect(canvas, page.col, page.row)
                .translate(self.page_jiggle_offset(page.col, page.row, time));
            if !paper.is_positive() {
                continue;
            }
            if hover.is_some_and(|p| paper.contains(p)) {
                painter.rect_stroke(
                    paper,
                    sheet_r,
                    Stroke::new(1.6, self.look.ink.gamma_multiply(0.65)),
                    StrokeKind::Inside,
                );
            }
        }
        let home = self.unit_screen_rect(canvas, drag.col, drag.row);
        if home.is_positive() {
            painter.rect_filled(
                home,
                6.0,
                Color32::from_rgba_unmultiplied(
                    self.look.desk.r(),
                    self.look.desk.g(),
                    self.look.desk.b(),
                    90,
                ),
            );
        }
        let ghost = Rect::from_min_size(drag.screen - drag.grab, home.size());
        painter.rect_filled(
            ghost.translate(vec2(0.0, 4.0)),
            8.0,
            Color32::from_rgba_unmultiplied(
                self.look.shadow.r(),
                self.look.shadow.g(),
                self.look.shadow.b(),
                70,
            ),
        );
        painter.rect_filled(
            ghost,
            8.0,
            self.look
                .paper_surface(note.paper_tone)
                .gamma_multiply(0.82),
        );
        painter.rect_stroke(
            ghost,
            8.0,
            Stroke::new(1.6, accent.gamma_multiply(0.85)),
            StrokeKind::Inside,
        );
    }

    fn paint_template(&self, painter: &Painter, paper: Rect, kind: PaperKind, scale: Vec2) {
        let sx = scale.x;
        let sy = scale.y;
        let tone = self
            .note
            .as_ref()
            .map_or(PaperTone::Default, |note| note.paper_tone);
        let rule = self.look.paper_rule(tone);
        let strong = self.look.paper_rule_strong(tone);
        let view = painter.clip_rect().expand(2.0);
        match kind {
            PaperKind::Blank => {}
            PaperKind::Lined | PaperKind::Cornell => {
                let cue = if kind == PaperKind::Cornell {
                    CORNELL_CUE * sx
                } else {
                    56.0 * sx
                };
                let summary = if kind == PaperKind::Cornell {
                    paper.max.y - CORNELL_SUMMARY * sy
                } else {
                    paper.max.y - 24.0 * sy
                };
                let mut y = paper.min.y + 88.0 * sy;
                while y < summary {
                    if y > view.max.y {
                        break;
                    }
                    if y >= view.min.y {
                        painter.line_segment(
                            [pos2(paper.min.x + cue, y), pos2(paper.max.x - 24.0 * sx, y)],
                            Stroke::new(1.0, rule),
                        );
                    }
                    y += 28.0 * sy;
                }
                let margin = self.margin_ink(tone);
                let x = paper.min.x
                    + if kind == PaperKind::Cornell {
                        CORNELL_CUE * sx
                    } else {
                        64.0 * sx
                    };
                let margin_top = paper.min.y + 24.0 * sy;
                if x >= view.min.x
                    && x <= view.max.x
                    && summary >= view.min.y
                    && margin_top <= view.max.y
                {
                    painter.line_segment(
                        [pos2(x, margin_top), pos2(x, summary)],
                        Stroke::new(1.15, margin),
                    );
                }
                if kind == PaperKind::Cornell && summary >= view.min.y && summary <= view.max.y {
                    painter.line_segment(
                        [
                            pos2(paper.min.x + 24.0 * sx, summary),
                            pos2(paper.max.x - 24.0 * sx, summary),
                        ],
                        Stroke::new(1.15, margin),
                    );
                }
            }
            PaperKind::Grid => {
                let mut x = paper.min.x + 24.0 * sx;
                while x < paper.max.x {
                    if x > view.max.x {
                        break;
                    }
                    if x >= view.min.x {
                        painter.line_segment(
                            [
                                pos2(x, paper.min.y + 24.0 * sy),
                                pos2(x, paper.max.y - 24.0 * sy),
                            ],
                            Stroke::new(0.8, rule),
                        );
                    }
                    x += 24.0 * sx;
                }
                let mut y = paper.min.y + 24.0 * sy;
                while y < paper.max.y {
                    if y > view.max.y {
                        break;
                    }
                    if y >= view.min.y {
                        painter.line_segment(
                            [
                                pos2(paper.min.x + 24.0 * sx, y),
                                pos2(paper.max.x - 24.0 * sx, y),
                            ],
                            Stroke::new(0.8, rule),
                        );
                    }
                    y += 24.0 * sy;
                }
            }
            PaperKind::Dots => {
                let radius = 1.6 * sx.min(sy).max(0.6);
                let dots = view.expand(radius);
                let mut y = paper.min.y + 32.0 * sy;
                while y < paper.max.y - 16.0 * sy {
                    if y > dots.max.y {
                        break;
                    }
                    if y >= dots.min.y {
                        let mut x = paper.min.x + 32.0 * sx;
                        while x < paper.max.x - 16.0 * sx {
                            if x > dots.max.x {
                                break;
                            }
                            if x >= dots.min.x {
                                painter.circle_filled(pos2(x, y), radius, strong);
                            }
                            x += 22.0 * sx;
                        }
                    }
                    y += 22.0 * sy;
                }
            }
            PaperKind::Millimetre => {
                let mut i = 0;
                let mut y = paper.min.y + 40.0 * sy;
                while y < paper.max.y - 20.0 * sy {
                    if y > view.max.y {
                        break;
                    }
                    if y >= view.min.y {
                        let c = if i % 5 == 0 { strong } else { rule };
                        painter.line_segment(
                            [
                                pos2(paper.min.x + 48.0 * sx, y),
                                pos2(paper.max.x - 20.0 * sx, y),
                            ],
                            Stroke::new(if i % 5 == 0 { 1.0 } else { 0.6 }, c),
                        );
                    }
                    y += 8.0 * sy;
                    i += 1;
                }
                let mut i = 0;
                let mut x = paper.min.x + 48.0 * sx;
                while x < paper.max.x - 20.0 * sx {
                    if x > view.max.x {
                        break;
                    }
                    if x >= view.min.x {
                        let c = if i % 5 == 0 { strong } else { rule };
                        painter.line_segment(
                            [
                                pos2(x, paper.min.y + 40.0 * sy),
                                pos2(x, paper.max.y - 20.0 * sy),
                            ],
                            Stroke::new(if i % 5 == 0 { 1.0 } else { 0.6 }, c),
                        );
                    }
                    x += 8.0 * sx;
                    i += 1;
                }
            }
        }
    }

    /// Linked sheets share one ruling, as if the block were a single larger page.
    /// Adjacent cells join into one stroke. A hole breaks the line.
    fn paint_fused_rules(&self, painter: &Painter, note: &Note, cam: Camera, canvas: Rect) {
        let kind = note.paper;
        if kind == PaperKind::Blank {
            return;
        }
        let cells: Vec<(i32, i32)> = note
            .pages
            .iter()
            .filter(|p| p.background.is_none())
            .map(|p| (p.col, p.row))
            .collect();
        if cells.is_empty() {
            return;
        }
        let pw = note.page_w;
        let ph = note.page_h;
        let min_c = cells.iter().map(|(c, _)| *c).min().unwrap_or(0);
        let max_c = cells.iter().map(|(c, _)| *c).max().unwrap_or(0);
        let min_r = cells.iter().map(|(_, r)| *r).min().unwrap_or(0);
        let max_r = cells.iter().map(|(_, r)| *r).max().unwrap_or(0);
        let ox = min_c as f32 * pw;
        let oy = min_r as f32 * ph;
        let bw = (max_c - min_c + 1) as f32 * pw;
        let bh = (max_r - min_r + 1) as f32 * ph;
        let rule = Stroke::new(1.0, self.look.paper_rule(note.paper_tone));
        let strong = Stroke::new(1.0, self.look.paper_rule_strong(note.paper_tone));
        let hair = Stroke::new(0.6, self.look.paper_rule(note.paper_tone));
        let margin = Stroke::new(1.15, self.margin_ink(note.paper_tone));

        let h =
            |painter: &Painter, y: f32, inset_l: f32, inset_r: f32, edge: f32, stroke: Stroke| {
                let row = rule_row(y, ph);
                for (start, end) in col_runs_on_row(&cells, row) {
                    let left = if start == min_c {
                        ox + inset_l
                    } else {
                        start as f32 * pw + edge
                    };
                    let right = if end == max_c {
                        ox + bw - inset_r
                    } else {
                        (end as f32 + 1.0) * pw - edge
                    };
                    if right > left + 1.0 {
                        let a = cam.to_screen(pos2(left, y), canvas);
                        let b = cam.to_screen(pos2(right, y), canvas);
                        if Rect::from_two_pos(a, b).intersects(painter.clip_rect().expand(2.0)) {
                            painter.line_segment([a, b], stroke);
                        }
                    }
                }
            };
        let v =
            |painter: &Painter, x: f32, inset_t: f32, inset_b: f32, edge: f32, stroke: Stroke| {
                let col = rule_row(x, pw);
                for (start, end) in row_runs_on_col(&cells, col) {
                    let top = if start == min_r {
                        oy + inset_t
                    } else {
                        start as f32 * ph + edge
                    };
                    let bot = if end == max_r {
                        oy + bh - inset_b
                    } else {
                        (end as f32 + 1.0) * ph - edge
                    };
                    if bot > top + 1.0 {
                        let a = cam.to_screen(pos2(x, top), canvas);
                        let b = cam.to_screen(pos2(x, bot), canvas);
                        if Rect::from_two_pos(a, b).intersects(painter.clip_rect().expand(2.0)) {
                            painter.line_segment([a, b], stroke);
                        }
                    }
                }
            };

        let clip = painter.clip_rect().expand(2.0);
        let screen_y = |y: f32| cam.to_screen(pos2(ox, y), canvas).y;
        let screen_x = |x: f32| cam.to_screen(pos2(x, oy), canvas).x;
        match kind {
            PaperKind::Blank => {}
            PaperKind::Lined => {
                let mut y = oy + 88.0;
                while y < oy + bh - 24.0 {
                    let sy = screen_y(y);
                    if sy > clip.max.y {
                        break;
                    }
                    if sy >= clip.min.y {
                        h(painter, y, 56.0, 24.0, 16.0, rule);
                    }
                    y += 28.0;
                }
                let sx = screen_x(ox + 64.0);
                if sx >= clip.min.x && sx <= clip.max.x {
                    v(painter, ox + 64.0, 24.0, 24.0, 0.0, margin);
                }
            }
            PaperKind::Cornell => {
                let summary = oy + bh - CORNELL_SUMMARY;
                let cue = ox + CORNELL_CUE;
                let mut y = oy + 88.0;
                while y < summary - 8.0 {
                    let sy = screen_y(y);
                    if sy > clip.max.y {
                        break;
                    }
                    if sy >= clip.min.y {
                        h(painter, y, CORNELL_CUE + 12.0, 24.0, 16.0, rule);
                    }
                    y += 28.0;
                }
                let sx = screen_x(cue);
                if sx >= clip.min.x && sx <= clip.max.x {
                    v(painter, cue, 24.0, CORNELL_SUMMARY, 0.0, margin);
                }
                let summary_y = screen_y(summary);
                if summary_y >= clip.min.y && summary_y <= clip.max.y {
                    h(painter, summary, 24.0, 24.0, 12.0, margin);
                }
            }
            PaperKind::Grid => {
                let mut x = ox + 24.0;
                while x < ox + bw - 24.0 {
                    let sx = screen_x(x);
                    if sx > clip.max.x {
                        break;
                    }
                    if sx >= clip.min.x {
                        v(painter, x, 24.0, 24.0, 12.0, rule);
                    }
                    x += 24.0;
                }
                let mut y = oy + 24.0;
                while y < oy + bh - 24.0 {
                    let sy = screen_y(y);
                    if sy > clip.max.y {
                        break;
                    }
                    if sy >= clip.min.y {
                        h(painter, y, 24.0, 24.0, 12.0, rule);
                    }
                    y += 24.0;
                }
            }
            PaperKind::Dots => {
                let dot = self.look.paper_rule_strong(note.paper_tone);
                let occupied: HashSet<(i32, i32)> = cells.iter().copied().collect();
                let radius = 1.6 * cam.zoom_min().max(0.6);
                let dots = clip.expand(radius);
                let mut y = oy + 32.0;
                while y < oy + bh - 16.0 {
                    if screen_y(y) > dots.max.y {
                        break;
                    }
                    if screen_y(y) >= dots.min.y {
                        let row = rule_row(y, ph);
                        let mut x = ox + 32.0;
                        while x < ox + bw - 16.0 {
                            let p = cam.to_screen(pos2(x, y), canvas);
                            if p.x > dots.max.x {
                                break;
                            }
                            if p.x >= dots.min.x {
                                let col = rule_row(x, pw);
                                if occupied.contains(&(col, row)) {
                                    painter.circle_filled(p, radius, dot);
                                }
                            }
                            x += 22.0;
                        }
                    }
                    y += 22.0;
                }
            }
            PaperKind::Millimetre => {
                let mut i = 0i32;
                let mut y = oy + 40.0;
                while y < oy + bh - 20.0 {
                    let sy = screen_y(y);
                    if sy > clip.max.y {
                        break;
                    }
                    if sy >= clip.min.y {
                        let stroke = if i % 5 == 0 { strong } else { hair };
                        h(painter, y, 48.0, 20.0, 8.0, stroke);
                    }
                    y += 8.0;
                    i += 1;
                }
                let mut i = 0i32;
                let mut x = ox + 48.0;
                while x < ox + bw - 20.0 {
                    let sx = screen_x(x);
                    if sx > clip.max.x {
                        break;
                    }
                    if sx >= clip.min.x {
                        let stroke = if i % 5 == 0 { strong } else { hair };
                        v(painter, x, 40.0, 20.0, 8.0, stroke);
                    }
                    x += 8.0;
                    i += 1;
                }
            }
        }

        if kind.has_margin_punches() {
            let z = cam.zoom_min().clamp(0.35, 3.0);
            let punch = if note.paper_tone == PaperTone::Black {
                Color32::from_gray(24)
            } else {
                self.look.punch
            };
            for k in 0..3 {
                let y = oy + bh * (0.22 + k as f32 * 0.28);
                let row = rule_row(y, ph);
                if !cells.contains(&(min_c, row)) {
                    continue;
                }
                let c = cam.to_screen(pos2(ox + 22.0, y), canvas);
                painter.circle_filled(c, 7.0 * z, punch);
                painter.circle_stroke(
                    c,
                    7.0 * z,
                    Stroke::new(1.0, self.look.paper_rule_strong(note.paper_tone)),
                );
                painter.circle_filled(c, 3.2 * z, self.look.desk);
            }
        }
    }

    fn margin_ink(&self, tone: PaperTone) -> Color32 {
        self.look.paper_margin(tone)
    }

    fn paint_punches(&self, painter: &Painter, paper: Rect, zoom: f32, tone: PaperTone) {
        let z = zoom.clamp(0.35, 3.0);
        let rule = self.look.paper_rule_strong(tone);
        let punch = if tone == PaperTone::Black {
            Color32::from_gray(24)
        } else {
            self.look.punch
        };
        for k in 0..3 {
            let y = paper.min.y + paper.height() * (0.22 + k as f32 * 0.28);
            let c = pos2(paper.min.x + 22.0 * z, y);
            painter.circle_filled(c, 7.0 * z, punch);
            painter.circle_stroke(c, 7.0 * z, Stroke::new(1.0, rule));
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
                        let size = vec2(tx.size[0] * cam.zoom, tx.size[1] * cam.zoom_y());
                        let size_pt = tx.size_pt;
                        let col = if tx.sticky {
                            self.look.ink
                        } else if note.paper_tone == PaperTone::Black
                            && tx.color32() == self.look.ink
                        {
                            Color32::WHITE
                        } else {
                            tx.color32()
                        };
                        Area::new(Id::new(("tx", id)))
                            .fixed_pos(min)
                            .order(Order::Foreground)
                            .constrain(false)
                            .show(ui.ctx(), |ui| {
                                ui.set_min_size(size);
                                let te = TextEdit::multiline(&mut tx.text)
                                    .id(Id::new(("tx-edit", id)))
                                    .font(self.look.serif(size_pt * cam.zoom_y()))
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
        if self.sheet_tabs_on {
            let hand = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| {
                !Self::pen_over_chrome(ui.ctx(), p)
                    && (self.sheet_tab_at(p, self.canvas_rect).is_some()
                        || self.page_minus_at(p, self.canvas_rect).is_some()
                        || self.sticky_minus_at(p, self.canvas_rect).is_some())
            });
            ui.ctx().set_cursor_icon(if hand {
                CursorIcon::PointingHand
            } else {
                CursorIcon::Grab
            });
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
            Tool::Hand => {
                if ui.input(|i| i.pointer.any_down()) {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                }
            }
            Tool::Lasso => CursorIcon::Crosshair,
            Tool::Image => CursorIcon::Copy,
            Tool::EraserArea => CursorIcon::None, // soft blur disk is the cursor
            Tool::EraserStroke => CursorIcon::NotAllowed,
            _ => CursorIcon::Crosshair,
        };
        ui.ctx().set_cursor_icon(icon);
    }

    /// Eraser brush size in screen pixels from the toolbar nib width.
    fn eraser_radius_screen(&self, area: bool) -> f32 {
        if area {
            (self.width * 1.8).clamp(6.0, 160.0)
        } else {
            (self.width * 1.15).clamp(4.0, 96.0)
        }
    }

    /// Paper-space radius: screen size / zoom, so zooming out covers more paper.
    fn eraser_radius_paper(&self, screen_per_paper: f32, area: bool) -> f32 {
        self.eraser_radius_screen(area) / screen_per_paper.max(1e-3)
    }

    fn area_eraser_active(&self, ctx: &Context) -> bool {
        if self.tool == Tool::EraserArea {
            return true;
        }
        let pen = self.tablet.snapshot();
        let secondary = ctx.input(|i| i.pointer.secondary_down());
        (pen.eraser || secondary) && self.last_eraser == Tool::EraserArea
    }

    /// Soft round blur disk under the pointer — only the zone, no chrome.
    fn paint_area_eraser_lens(&self, ui: &Ui, painter: &Painter, canvas: Rect) {
        self.paint_area_eraser_lens_at(ui.ctx(), painter, canvas);
    }

    fn paint_area_eraser_lens_at(&self, ctx: &Context, painter: &Painter, clip: Rect) {
        if !self.area_eraser_active(ctx) {
            return;
        }
        let pen = self.tablet.snapshot();
        let Some(screen) = pen
            .pos
            .filter(|_| pen.in_proximity || pen.down)
            .or_else(|| ctx.pointer_hover_pos())
        else {
            return;
        };
        if !clip.contains(screen) || Self::pen_over_chrome(ctx, screen) {
            return;
        }
        paint_soft_blur_disk(painter, screen, self.eraser_radius_screen(true));
    }

    fn pen_over_chrome(ctx: &Context, pos: Pos2) -> bool {
        ctx.data(|d| {
            d.get_temp::<Rect>(Id::new("selection-actions"))
                .is_some_and(|r| r.contains(pos))
                || d.get_temp::<Rect>(Id::new("pages-strip-rect"))
                    .map(|r| r.expand(6.0).contains(pos))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("dock-rect"))
                    .map(|r| r.expand(8.0).contains(pos))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("top-rect"))
                    .map(|r| r.expand(4.0).contains(pos))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("tin-rect"))
                    .map(|r| r.expand(4.0).contains(pos))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("nib-rect"))
                    .map(|r| r.expand(4.0).contains(pos))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("coach-button-rect"))
                    .map(|r| r.expand(4.0).contains(pos))
                    .unwrap_or(false)
        })
    }

    /// Stylus to egui pointer: hover and clicks on the chrome (pencil case, shelf).
    fn inject_pen_pointer(&mut self, ctx: &Context, raw: &mut egui::RawInput) {
        let pen = self.tablet.snapshot();
        // Hovering a stylus must not move the UI pointer away from a finger
        // tapping a toolbar button. Actual pen contact still owns pen input.
        if !pen.down
            && !pen.pressed
            && !pen.released
            && (ctx.input(|i| i.any_touches())
                || raw.events.iter().any(|e| matches!(e, Event::Touch { .. })))
        {
            return;
        }
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

    fn ingest_pdf_rasters(&mut self, ctx: &Context) {
        let Some(hub) = self.pdf_raster.as_ref() else {
            return;
        };
        hub.set_repaint_context(ctx);
        let ready = hub.take_ready();
        if !ready.is_empty() {
            ctx.request_repaint();
        }
        for (key, result) in ready {
            if !self.bg_pending.remove(&key) {
                continue;
            }
            match result {
                Ok(rendered) => {
                    self.store_background(ctx, key, rendered);
                }
                Err(_) => {
                    self.bg_failed.insert(key);
                }
            }
        }
    }

    fn store_background(
        &mut self,
        ctx: &Context,
        key: String,
        rendered: crate::pdf::RasterImage,
    ) -> Option<TextureHandle> {
        if rendered.image.width() == 0
            || rendered.image.height() == 0
            || rendered.image.pixels.is_empty()
        {
            self.bg_failed.insert(key);
            return None;
        }
        const BUDGET: usize = 96 * 1024 * 1024;
        let bytes = rendered.image.pixels.len() * 4;
        if self
            .bg_tex_scale
            .get(&key)
            .is_some_and(|have| *have >= rendered.scale)
        {
            return self.bg_textures.get(&key).cloned();
        }
        let old_bytes = self
            .bg_order
            .iter()
            .find(|(k, _)| k == &key)
            .map(|(_, b)| *b)
            .unwrap_or(0);
        let mut protect = self.bg_hot.clone();
        protect.insert(key.clone());
        while self.bg_bytes + bytes > BUDGET + old_bytes {
            let Some(i) = self.bg_order.iter().position(|(k, _)| !protect.contains(k)) else {
                break;
            };
            self.forget_background_at(i);
        }
        // The LRU budget is soft for visible sheets. Refusing their upgrades
        // would requeue the same expensive render every frame. Each raster is
        // independently bounded, and off-screen textures are evicted first.
        let tex = ctx.load_texture(key.clone(), rendered.image, TextureOptions::LINEAR);
        if let Some(i) = self.bg_order.iter().position(|(k, _)| k == &key) {
            self.forget_background_at(i);
        }
        self.bg_bytes += bytes;
        self.bg_order.push_back((key.clone(), bytes));
        self.bg_tex_scale.insert(key.clone(), rendered.scale);
        self.bg_textures.insert(key, tex.clone());
        Some(tex)
    }

    fn forget_background_at(&mut self, i: usize) {
        let Some((old, old_bytes)) = self.bg_order.remove(i) else {
            return;
        };
        if self.bg_textures.remove(&old).is_some() {
            self.bg_bytes = self.bg_bytes.saturating_sub(old_bytes);
        }
        self.bg_tex_scale.remove(&old);
    }

    fn background_tex(
        &mut self,
        ctx: &Context,
        note_id: Uuid,
        bg: &PageBackground,
        screen: Rect,
        pixels_per_point: f32,
    ) -> Option<TextureHandle> {
        let [src_w, src_h] = bg.source_size.unwrap_or([PAGE_W, PAGE_H]);
        let wanted = if bg.pdf_page.is_some() {
            let raw = crate::pdf::raster_scale(
                screen.width(),
                screen.height(),
                src_w,
                src_h,
                pixels_per_point,
            );
            crate::pdf::clamp_scale(src_w, src_h, raw)
                .min((ctx.input(|i| i.max_texture_side) as f32 - 1.0) / src_w.max(src_h).max(1.0))
        } else {
            1.0
        };
        let key = format!("{note_id}:{}:{}", bg.file, bg.pdf_page.unwrap_or(u32::MAX));
        self.bg_hot.insert(key.clone());
        let have = self.bg_tex_scale.get(&key).copied().unwrap_or(0.0);
        let cached = self.bg_textures.get(&key).cloned();
        if cached.is_some() {
            if let Some(pos) = self.bg_order.iter().position(|(k, _)| k == &key) {
                if let Some(entry) = self.bg_order.remove(pos) {
                    self.bg_order.push_back(entry);
                }
            }
        }
        if cached.is_some() && crate::pdf::raster_is_sharp_enough(have, wanted) {
            return cached;
        }
        if self.bg_failed.contains(&key) || self.bg_pending.contains(&key) {
            return cached;
        }
        let path = self.lib.note_dir(note_id).join(&bg.file);
        if let Some(page) = bg.pdf_page {
            let delay = self.pdf_detail_after - ctx.input(|i| i.time);
            if cached.is_some() && delay > 0.0 {
                ctx.request_repaint_after(Duration::from_secs_f64(delay));
                return cached;
            }
            let hub = self
                .pdf_raster
                .get_or_insert_with(crate::pdf::RasterHub::new);
            hub.set_repaint_context(ctx);
            if hub.request(crate::pdf::RasterJob {
                key: key.clone(),
                path,
                page,
                scale: wanted,
                preview: cached.is_none(),
            }) {
                self.bg_pending.insert(key);
            }
            return cached;
        }
        let rendered = image::open(&path).ok().and_then(|img| {
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
                    scale: 1.0,
                })
            }
        });
        let Some(rendered) = rendered else {
            self.bg_failed.insert(key);
            return cached;
        };
        self.store_background(ctx, key, (&rendered).into())
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

/// Stroke ribbon covers the view, including the round caps past the point box.
fn ink_visible(stroke: &InkStroke, ox: f32, oy: f32, sx: f32, sy: f32, view: Rect) -> bool {
    let Some((min, max)) = stroke.bbox() else {
        return false;
    };
    let pad_x = stroke.width * 0.75 * sx + 1.0;
    let pad_y = stroke.width * 0.75 * sy + 1.0;
    Rect::from_min_max(
        pos2(ox + min.x * sx - pad_x, oy + min.y * sy - pad_y),
        pos2(ox + max.x * sx + pad_x, oy + max.y * sy + pad_y),
    )
    .intersects(view)
}

fn dock_rest_min(edge: DockEdge, size: Vec2, bounds: Rect) -> Pos2 {
    let (align, off) = match edge {
        DockEdge::Bottom => (Align2::CENTER_BOTTOM, vec2(0.0, -4.0)),
        DockEdge::Top => (Align2::CENTER_TOP, Vec2::ZERO),
        DockEdge::Left => (Align2::LEFT_CENTER, vec2(2.0, 0.0)),
        DockEdge::Right => (Align2::RIGHT_CENTER, vec2(-2.0, 0.0)),
    };
    let mut min = align.align_size_within_rect(size, bounds).min + off;
    let margin_x = (size.x - bounds.width()).max(0.0);
    let margin_y = (size.y - bounds.height()).max(0.0);
    min.x = min
        .x
        .min(bounds.right() + margin_x - size.x)
        .max(bounds.left() - margin_x);
    min.y = min
        .y
        .min(bounds.bottom() + margin_y - size.y)
        .max(bounds.top() - margin_y);
    min
}

fn clamp_speed(v: Vec2, max: f32) -> Vec2 {
    let speed = v.length();
    if speed > max && speed > 0.0 {
        v * (max / speed)
    } else {
        v
    }
}

fn edge_distance(pos: Pos2, screen: Rect, edge: DockEdge) -> f32 {
    match edge {
        DockEdge::Top => pos.y - screen.top(),
        DockEdge::Bottom => screen.bottom() - pos.y,
        DockEdge::Left => pos.x - screen.left(),
        DockEdge::Right => screen.right() - pos.x,
    }
}

/// Smoothstep onto the wall. The along-edge axis stays on the finger.
fn dock_magnet(dist: f32) -> f32 {
    let t = (1.0 - dist / DOCK_MAGNET).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Side the case is heading for.
///
/// A change of axis happens before the long side meets that wall. Waiting for
/// the pointer to enter the corner let the clamp pile the bar up first.
fn carrying_edge(
    pos: Pos2,
    screen: Rect,
    bounds: Rect,
    current: DockEdge,
    body: Rect,
    vel: Vec2,
) -> DockEdge {
    if let Some(edge) = imminent_wall(body, vel, bounds, current) {
        return edge;
    }
    let next = snap_dock(pos, screen);
    if next.vertical() == current.vertical() {
        return next;
    }
    // The long side already turned. The pointer stays on the edge it left for
    // the rest of the trip, so a nearest-edge snap would stand the case back
    // up against that wall. Take the old edge again only when the hand moves
    // into it.
    if !heading_into(vel, next) {
        return current;
    }
    let dist = edge_distance(pos, screen, next);
    if dock_magnet(dist) >= DOCK_TURN && dist + 28.0 < edge_distance(pos, screen, current) {
        next
    } else {
        current
    }
}

/// Perpendicular wall the long side is about to meet, if the hand is moving
/// into it. The grip stays under the finger, so the case swings before contact.
fn imminent_wall(body: Rect, vel: Vec2, bounds: Rect, current: DockEdge) -> Option<DockEdge> {
    let edges = if current.vertical() {
        [DockEdge::Top, DockEdge::Bottom]
    } else {
        [DockEdge::Left, DockEdge::Right]
    };
    let mut best: Option<(DockEdge, f32)> = None;
    for edge in edges {
        let clear = side_clearance(body, bounds, edge);
        if clear < DOCK_TURN_AIR
            && heading_into(vel, edge)
            && best.is_none_or(|(_, have)| clear < have)
        {
            best = Some((edge, clear));
        }
    }
    best.map(|(edge, _)| edge)
}

fn side_clearance(body: Rect, bounds: Rect, edge: DockEdge) -> f32 {
    match edge {
        DockEdge::Left => body.left() - bounds.left(),
        DockEdge::Right => bounds.right() - body.right(),
        DockEdge::Top => body.top() - bounds.top(),
        DockEdge::Bottom => bounds.bottom() - body.bottom(),
    }
}

fn heading_into(vel: Vec2, edge: DockEdge) -> bool {
    let into = match edge {
        DockEdge::Right => vel.x,
        DockEdge::Left => -vel.x,
        DockEdge::Bottom => vel.y,
        DockEdge::Top => -vel.y,
    };
    let lateral = match edge {
        DockEdge::Left | DockEdge::Right => vel.y.abs(),
        DockEdge::Top | DockEdge::Bottom => vel.x.abs(),
    };
    into > 120.0 && into >= lateral
}

fn magnet_min(hand: Pos2, rest: Pos2, edge: DockEdge, pull: f32) -> Pos2 {
    match edge {
        DockEdge::Top | DockEdge::Bottom => pos2(hand.x, hand.y + (rest.y - hand.y) * pull),
        DockEdge::Left | DockEdge::Right => pos2(hand.x + (rest.x - hand.x) * pull, hand.y),
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

struct InkWheelGeom {
    center: Pos2,
    sv_r: f32,
    ring_inner: f32,
    ring_outer: f32,
}

fn ink_wheel_geom(rect: Rect) -> InkWheelGeom {
    let outer = rect.width().min(rect.height()) * 0.5 - 1.0;
    let ring_inner = (outer - TIN_RING).max(outer * 0.55);
    let sv_r = (ring_inner - TIN_RING_GAP).max(8.0);
    InkWheelGeom {
        center: rect.center(),
        sv_r,
        ring_inner,
        ring_outer: outer,
    }
}

impl InkWheelGeom {
    fn sv_pos(&self, s: f32, v: f32) -> Pos2 {
        pos2(
            self.center.x + (s * 2.0 - 1.0) * self.sv_r,
            self.center.y + (1.0 - v) * 2.0 * self.sv_r - self.sv_r,
        )
    }

    fn hue_pos(&self, hue: f32) -> Pos2 {
        let a = hue * std::f32::consts::TAU;
        let r = (self.ring_inner + self.ring_outer) * 0.5;
        self.center + vec2(a.cos(), a.sin()) * r
    }
}

/// 0 at a hairline, 1 at the heavy end. The curve leaves more room for fine nibs.
fn nib_t(width: f32) -> f32 {
    let u = ((width - NIB_MIN) / (NIB_MAX - NIB_MIN)).clamp(0.0, 1.0);
    u.sqrt()
}

fn width_from_nib_t(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    NIB_MIN + (NIB_MAX - NIB_MIN) * t * t
}

fn nib_track_t(track: Rect, p: Pos2) -> f32 {
    ((p.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0)
}

fn nib_handle_pos(track: Rect, t: f32) -> Pos2 {
    let t = t.clamp(0.0, 1.0);
    pos2(track.left() + track.width() * t, track.center().y)
}

fn paint_disc_shadow(p: &Painter, c: Pos2, r: f32, shadow: Color32) {
    let soft = Color32::from_rgba_unmultiplied(shadow.r(), shadow.g(), shadow.b(), 32);
    let deep = Color32::from_rgba_unmultiplied(shadow.r(), shadow.g(), shadow.b(), 56);
    p.circle_filled(c + vec2(0.0, 6.0), r * 0.94, soft);
    p.circle_filled(c + vec2(0.0, 2.5), r * 0.9, deep);
}

fn paint_sv_disk(p: &Painter, c: Pos2, r: f32, hue: f32) {
    let rings = 18;
    let segs = 64;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c, hsv_to_rgb(hue, 0.5, 0.5));
    for ring in 1..=rings {
        let rad = r * ring as f32 / rings as f32;
        for i in 0..segs {
            let a = i as f32 / segs as f32 * std::f32::consts::TAU;
            let (sin, cos) = a.sin_cos();
            let x = cos * rad;
            let y = sin * rad;
            let s = (x / r * 0.5 + 0.5).clamp(0.0, 1.0);
            let v = (0.5 - y / r * 0.5).clamp(0.0, 1.0);
            mesh.colored_vertex(c + vec2(x, y), hsv_to_rgb(hue, s, v));
        }
    }
    for i in 0..segs {
        let next = (i + 1) % segs;
        mesh.indices.extend([0, 1 + i, 1 + next]);
    }
    for ring in 1..rings {
        let inner = 1 + (ring - 1) * segs;
        let outer = 1 + ring * segs;
        for i in 0..segs {
            let next = (i + 1) % segs;
            mesh.indices.extend([
                inner + i,
                outer + i,
                outer + next,
                inner + i,
                outer + next,
                inner + next,
            ]);
        }
    }
    p.add(Shape::mesh(mesh));
}

fn paint_hue_ring(p: &Painter, c: Pos2, r_in: f32, r_out: f32) {
    let segs = 72;
    let mut mesh = Mesh::default();
    for i in 0..=segs {
        let t = i as f32 / segs as f32;
        let a = t * std::f32::consts::TAU;
        let (sin, cos) = a.sin_cos();
        let color = hsv_to_rgb(t, 1.0, 1.0);
        mesh.colored_vertex(c + vec2(cos, sin) * r_out, color);
        mesh.colored_vertex(c + vec2(cos, sin) * r_in, color);
        if i < segs {
            let i = i * 2;
            mesh.indices.extend([i, i + 2, i + 3, i, i + 3, i + 1]);
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

fn sticky_pad_place(dock: Rect, edge: DockEdge, bounds: Rect) -> Rect {
    let size = vec2(48.0, 48.0);
    let gap = 8.0;
    let candidates = match edge {
        DockEdge::Bottom => [
            pos2(dock.max.x + gap, dock.center().y - size.y * 0.5),
            pos2(dock.max.x - size.x, dock.min.y - gap - size.y),
            pos2(dock.min.x - gap - size.x, dock.center().y - size.y * 0.5),
        ],
        DockEdge::Top => [
            pos2(dock.max.x + gap, dock.center().y - size.y * 0.5),
            pos2(dock.max.x - size.x, dock.max.y + gap),
            pos2(dock.min.x - gap - size.x, dock.center().y - size.y * 0.5),
        ],
        DockEdge::Left => [
            pos2(dock.center().x - size.x * 0.5, dock.max.y + gap),
            pos2(dock.max.x + gap, dock.max.y - size.y),
            pos2(dock.center().x - size.x * 0.5, dock.min.y - gap - size.y),
        ],
        DockEdge::Right => [
            pos2(dock.center().x - size.x * 0.5, dock.max.y + gap),
            pos2(dock.min.x - gap - size.x, dock.max.y - size.y),
            pos2(dock.center().x - size.x * 0.5, dock.min.y - gap - size.y),
        ],
    };
    let clamp = |p: Pos2| {
        pos2(
            p.x.clamp(bounds.min.x, (bounds.max.x - size.x).max(bounds.min.x)),
            p.y.clamp(bounds.min.y, (bounds.max.y - size.y).max(bounds.min.y)),
        )
    };
    let keepout = dock.expand(4.0);
    candidates
        .into_iter()
        .map(|p| Rect::from_min_size(clamp(p), size))
        .find(|r| bounds.contains_rect(*r) && !r.intersects(keepout))
        .unwrap_or_else(|| Rect::from_min_size(clamp(candidates[0]), size))
}

/// A note with a soft drop, a lit top edge, and a folded corner.
fn paint_sticky_card(p: &Painter, rect: Rect, color: Color32, shadow: Color32) {
    if rect.width() < 4.0 || rect.height() < 4.0 {
        return;
    }
    let body = Color32::from_rgb(color.r(), color.g(), color.b());
    let short = rect.width().min(rect.height());
    let radius = (short * 0.035).clamp(1.5, 5.0);
    let drop = (short * 0.055).clamp(2.5, 7.0);
    p.rect_filled(
        rect.translate(vec2(drop * 0.45, drop)).expand(drop * 0.28),
        radius + 1.5,
        Color32::from_rgba_unmultiplied(shadow.r(), shadow.g(), shadow.b(), 42),
    );
    p.rect_filled(
        rect.translate(vec2(drop * 0.12, drop * 0.42)),
        radius,
        Color32::from_rgba_unmultiplied(shadow.r(), shadow.g(), shadow.b(), 96),
    );
    p.rect_filled(rect, radius, body);
    let lip = (short * 0.07).clamp(2.0, 8.0);
    p.rect_filled(
        Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + lip)),
        radius,
        mix_col(body, Color32::WHITE, 0.42).gamma_multiply(0.55),
    );
    p.rect_filled(
        Rect::from_min_max(pos2(rect.min.x, rect.max.y - lip * 0.65), rect.max),
        0.0,
        shade_rgb(body, 0.78).gamma_multiply(0.28),
    );
    let ear = (short * 0.2).clamp(10.0, 26.0);
    let tip = rect.right_bottom();
    let along_bottom = tip + vec2(-ear, 0.0);
    let along_side = tip + vec2(0.0, -ear);
    let flap = tip + vec2(-ear * 0.72, -ear * 0.72);
    p.add(Shape::convex_polygon(
        vec![along_bottom, tip, along_side],
        shade_rgb(body, 0.62),
        Stroke::NONE,
    ));
    p.add(Shape::convex_polygon(
        vec![along_bottom, flap, along_side],
        mix_col(body, Color32::WHITE, 0.22),
        Stroke::new(1.05, shade_rgb(body, 0.7)),
    ));
    p.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, shade_rgb(body, 0.72)),
        StrokeKind::Inside,
    );
}

fn sticky_minus_band(note: Rect) -> Rect {
    let along = (note.width() * 0.72).min(note.width() - 6.0);
    let thick = (note.height() * 0.32).min(note.height() - 6.0);
    if along < 8.0 || thick < 8.0 {
        return Rect::NOTHING;
    }
    Rect::from_center_size(note.center(), vec2(along, thick))
}

fn image_minus_band(image: Rect) -> Rect {
    Rect::from_center_size(image.right_top() + vec2(-7.0, 7.0), vec2(40.0, 40.0))
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
/// Ignore taps before showing a long-press indicator.
const HOLD_RING_DELAY: f64 = 0.18;
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

fn paint_cloth_wheel(
    p: &Painter,
    origin: Pos2,
    r0: f32,
    r1: f32,
    n_cloth: usize,
    hover: Option<WheelPick>,
    look: &Look,
) {
    for i in 0..n_cloth {
        let cloth = look.cloth_at(i as u8);
        let hot = hover == Some(WheelPick::Cloth(i as u8));
        let fill = if hot { shade_rgb(cloth, 1.22) } else { cloth };
        p.add(wedge_mesh(origin, r0, r1, n_cloth, i, fill, None));
        if hot {
            p.add(wedge_stroke(origin, r0, r1, n_cloth, i, look.ink));
        }
    }
    p.circle_filled(origin, r0 - 1.0, look.desk);
    p.circle_stroke(
        origin,
        r0,
        Stroke::new(1.4_f32, look.ink.gamma_multiply(0.28)),
    );
    p.circle_stroke(
        origin,
        r1,
        Stroke::new(1.4_f32, look.ink.gamma_multiply(0.22)),
    );
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
    let i = ((a / std::f32::consts::TAU) * 3.0).floor() as usize % 3;
    Some([WheelPick::Color, WheelPick::Trash, WheelPick::Mark][i])
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

/// Stylus and finger taps wander. A page mark still commits inside this
/// distance; a longer drag pans and does not add or delete.
const PAGE_MARK_SLOP: f32 = 36.0;
/// Share of the page edge a `+` or `−` covers, and its thickness.
const MARK_ALONG: f32 = 0.34;
const MARK_THICK: f32 = 0.12;
const MARK_GAP: f32 = 0.06;
/// Centered on the edge, set off from the sheet. Same footprint as the center `−`.
fn outward_band(paper: Rect, edge: SheetEdge) -> Rect {
    let gap = paper.height() * MARK_GAP;
    let thick = paper.height() * MARK_THICK;
    let along_h = paper.height() * MARK_ALONG;
    let along_w = paper.width() * MARK_ALONG;
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

/// Same footprint as a top or bottom `+`, sitting on the page center.
fn page_minus_band(paper: Rect) -> Rect {
    let along = paper.width() * MARK_ALONG;
    let thick = paper.height() * MARK_THICK;
    Rect::from_center_size(paper.center(), vec2(along, thick))
}

fn center_band(paper: Rect) -> Rect {
    let w = paper.width() * 0.42;
    let h = paper.height() * 0.28;
    Rect::from_center_size(paper.center(), vec2(w, h))
}

fn sheet_mark_colors(pigment: Color32, paper: Color32) -> (Color32, Color32, Color32) {
    let fill = Color32::from_rgba_unmultiplied(pigment.r(), pigment.g(), pigment.b(), 255);
    let hot = mix_col(fill, Color32::WHITE, 0.16);
    (fill, hot, paper)
}

fn paint_sheet_tab(
    p: &Painter,
    rect: Rect,
    fill: Color32,
    glyph: Color32,
    shadow: Color32,
    minus: bool,
) {
    let short = rect.width().min(rect.height());
    let radius = short * 0.22;
    let drop = (short * 0.08).clamp(2.0, 5.0);
    p.rect_filled(
        rect.translate(vec2(0.0, drop)),
        radius,
        Color32::from_rgba_unmultiplied(shadow.r(), shadow.g(), shadow.b(), shadow.a().max(120)),
    );
    p.rect_filled(rect, radius, fill);
    p.rect_stroke(
        rect,
        radius,
        Stroke::new((short * 0.045).clamp(1.25, 2.2), shade_rgb(fill, 0.62)),
        StrokeKind::Inside,
    );
    let c = rect.center();
    let arm = short * 0.34;
    let thick = (short * 0.075).max(1.6);
    p.rect_filled(
        Rect::from_center_size(c, vec2(arm * 2.0, thick)),
        thick * 0.5,
        glyph,
    );
    if !minus {
        p.rect_filled(
            Rect::from_center_size(c, vec2(thick, arm * 2.0)),
            thick * 0.5,
            glyph,
        );
    }
}

/// Soft round blur disk for the area eraser — concentric washes, no rim.
fn paint_soft_blur_disk(p: &Painter, c: Pos2, radius: f32) {
    const RINGS: i32 = 14;
    for i in (1..=RINGS).rev() {
        let t = i as f32 / RINGS as f32;
        let r = radius * t;
        // Stronger in the middle, feathered to nothing at the edge.
        let fall = (1.0 - t).powi(2);
        let a = (fall * 42.0).round() as u8;
        if a < 2 {
            continue;
        }
        p.circle_filled(c, r, Color32::from_rgba_unmultiplied(255, 255, 255, a));
    }
}

/// Up arrow in the lectern tray.
fn paint_open_icon(p: &Painter, c: Pos2, fg: Color32) {
    let st = Stroke::new(1.8, fg);
    let o = c + vec2(0.0, -0.2);
    p.line_segment([o + vec2(0.0, 4.2), o + vec2(0.0, -3.0)], st);
    p.add(Shape::convex_polygon(
        vec![
            o + vec2(0.0, -8.8),
            o + vec2(-4.8, -2.6),
            o + vec2(4.8, -2.6),
        ],
        fg,
        Stroke::NONE,
    ));
    p.add(Shape::line(
        vec![
            o + vec2(-8.6, 1.6),
            o + vec2(-8.6, 8.0),
            o + vec2(8.6, 8.0),
            o + vec2(8.6, 1.6),
        ],
        st,
    ));
}

fn paint_plus(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.rect_filled(Rect::from_center_size(c, vec2(16.0, 3.0)), 2.0, fg);
    p.rect_filled(Rect::from_center_size(c, vec2(3.0, 16.0)), 2.0, fg);
}

/// Tiny page-grid glyph used by the pages-edit tray.
fn paint_grid_icon(p: &Painter, c: Pos2, cols: u8, rows: u8, fg: Color32) {
    paint_grid_icon_sized(p, c, cols, rows, 5.0, 2.0, fg);
}

fn paint_grid_icon_sized(
    p: &Painter,
    c: Pos2,
    cols: u8,
    rows: u8,
    cell: f32,
    gap: f32,
    fg: Color32,
) {
    let w = (cols.max(1) as f32) * cell + (cols.saturating_sub(1) as f32) * gap;
    let h = (rows.max(1) as f32) * cell + (rows.saturating_sub(1) as f32) * gap;
    let start = c - vec2(w * 0.5, h * 0.5);
    for row in 0..rows {
        for col in 0..cols {
            let page = Rect::from_min_size(
                start + vec2(col as f32 * (cell + gap), row as f32 * (cell + gap)),
                vec2(cell, cell),
            );
            p.rect_stroke(page, 1.0, Stroke::new(1.1, fg), StrokeKind::Inside);
        }
    }
}

/// Fused sheets share an edge.
fn paint_join_linked(p: &Painter, c: Pos2, fg: Color32, scale: f32) {
    let sheet = Rect::from_center_size(c, vec2(26.0, 16.0) * scale);
    p.rect_stroke(sheet, 2.0, Stroke::new(1.3, fg), StrokeKind::Inside);
    p.vline(
        sheet.center().x,
        sheet.y_range().shrink(3.0 * scale),
        Stroke::new(1.0, fg.gamma_multiply(0.45)),
    );
}

/// Sheets with a gutter between them.
fn paint_join_separate(p: &Painter, c: Pos2, fg: Color32, scale: f32) {
    for dx in [-11.0 * scale, 11.0 * scale] {
        p.rect_stroke(
            Rect::from_center_size(c + vec2(dx, 0.0), vec2(13.0, 16.0) * scale),
            2.0,
            Stroke::new(1.3, fg),
            StrokeKind::Inside,
        );
    }
}

fn paint_fullscreen_icon(p: &Painter, c: Pos2, fg: Color32) {
    let stroke = Stroke::new(1.8, fg);
    for sx in [-1.0, 1.0] {
        for sy in [-1.0, 1.0] {
            let corner = c + vec2(sx * 8.0, sy * 8.0);
            p.line_segment([corner - vec2(sx * 5.0, 0.0), corner], stroke);
            p.line_segment([corner - vec2(0.0, sy * 5.0), corner], stroke);
        }
    }
}

fn paint_search_icon(p: &Painter, c: Pos2, fg: Color32) {
    p.circle_stroke(c - vec2(2.5, 2.5), 6.0, Stroke::new(1.8, fg));
    p.line_segment(
        [c + vec2(1.8, 1.8), c + vec2(7.0, 7.0)],
        Stroke::new(2.0, fg),
    );
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

/// Open hand for selecting, moving and resizing stationery on the page.
fn paint_hand_icon(p: &Painter, c: Pos2, fg: Color32) {
    let st = Stroke::new(1.65, fg);
    let o = c + vec2(0.0, 1.0);
    p.add(Shape::line(
        vec![
            o + vec2(-7.5, -1.0),
            o + vec2(-7.5, -5.7),
            o + vec2(-4.7, -6.2),
            o + vec2(-3.8, 0.3),
            o + vec2(-3.8, -10.2),
            o + vec2(-0.9, -10.7),
            o + vec2(-0.5, -0.8),
            o + vec2(-0.2, -12.2),
            o + vec2(2.8, -12.2),
            o + vec2(2.6, -0.7),
            o + vec2(3.5, -9.5),
            o + vec2(6.3, -9.1),
            o + vec2(6.0, 3.2),
            o + vec2(2.7, 10.5),
            o + vec2(-3.0, 10.5),
            o + vec2(-7.5, 4.1),
            o + vec2(-7.5, -1.0),
        ],
        st,
    ));
}

/// Raised graduation cap, drawn as paper-and-ink chrome rather than an emoji.
fn paint_coach_icon(
    p: &Painter,
    c: Pos2,
    ink: Color32,
    accent: Color32,
    paper: Color32,
    shadow: Color32,
    enabled: bool,
) {
    let top = [
        c + vec2(-15.0, -4.0),
        c + vec2(0.0, -12.0),
        c + vec2(15.0, -4.0),
        c + vec2(0.0, 4.0),
    ];
    let shade = mix_col(ink, accent, 0.24);
    let light = mix_col(accent, paper, 0.38);
    let rim = Stroke::new(1.2, ink.gamma_multiply(0.85));
    p.add(Shape::convex_polygon(
        top.iter().map(|point| *point + vec2(2.0, 4.0)).collect(),
        shadow.gamma_multiply(0.55),
        Stroke::NONE,
    ));
    p.add(Shape::convex_polygon(
        vec![
            top[0],
            top[3],
            top[3] + vec2(0.0, 3.5),
            top[0] + vec2(0.0, 3.5),
        ],
        shade,
        Stroke::NONE,
    ));
    p.add(Shape::convex_polygon(
        vec![
            top[3],
            top[2],
            top[2] + vec2(0.0, 3.5),
            top[3] + vec2(0.0, 3.5),
        ],
        ink,
        Stroke::NONE,
    ));
    p.add(Shape::convex_polygon(top.to_vec(), light, rim));
    p.line_segment(
        [c + vec2(-12.0, -4.0), c + vec2(0.0, -10.0)],
        Stroke::new(1.1, paper.gamma_multiply(0.8)),
    );
    p.add(Shape::convex_polygon(
        vec![
            c + vec2(-7.0, 3.0),
            c + vec2(0.0, 6.5),
            c + vec2(0.0, 11.0),
            c + vec2(-7.0, 8.0),
        ],
        shade,
        rim,
    ));
    p.add(Shape::convex_polygon(
        vec![
            c + vec2(0.0, 6.5),
            c + vec2(7.0, 3.0),
            c + vec2(7.0, 8.0),
            c + vec2(0.0, 11.0),
        ],
        ink,
        rim,
    ));
    let tassel = if enabled { accent } else { ink };
    p.line_segment(
        [c + vec2(14.0, -4.0), c + vec2(14.0, 7.0)],
        Stroke::new(1.7, tassel),
    );
    p.line_segment(
        [c + vec2(12.5, 10.0), c + vec2(14.0, 7.0)],
        Stroke::new(1.8, tassel),
    );
    p.line_segment(
        [c + vec2(14.0, 7.0), c + vec2(15.5, 10.0)],
        Stroke::new(1.8, tassel),
    );
}

fn paint_tool_icon(p: &Painter, c: Pos2, fg: Color32, tool: Tool) {
    let stroke = Stroke::new(1.7, fg);
    match tool {
        Tool::Text => paint_text_icon(p, c, fg),
        Tool::Image => paint_image_icon(p, c, fg),
        Tool::Hand => paint_hand_icon(p, c, fg),
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
}
