use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::document::Note;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoteMeta {
    pub id: Uuid,
    pub title: String,
    pub updated: DateTime<Utc>,
    pub pinned: bool,
    pub cover: u8,
    #[serde(default)]
    pub emoji: String,
    /// Shelf cell (gaps allowed), 0..SHELF_SLOTS — or 0..TRASH_SLOTS while in the bin.
    #[serde(default)]
    pub slot: u32,
}

/// Shelf grid: 10 columns × 3 rows (fills a 16:9 lectern).
pub const SHELF_COLS: u32 = 10;
pub const SHELF_ROWS: u32 = 3;
pub const SHELF_SLOTS: u32 = SHELF_COLS * SHELF_ROWS;
/// One red row under the shelf, same 10 columns.
pub const TRASH_SLOTS: u32 = SHELF_COLS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockEdge {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

impl DockEdge {
    pub fn vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// Old index field (lectern / tablet). Kept so existing JSON still loads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteMode {
    #[default]
    #[serde(alias = "main")]
    Pupitre,
    #[serde(alias = "stylus", alias = "chiffon")]
    Tablette,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Index {
    pub seeded: bool,
    pub notes: Vec<NoteMeta>,
    #[serde(default)]
    pub trash: Vec<NoteMeta>,
    #[serde(default)]
    pub dock: DockEdge,
    #[serde(default, alias = "hand")]
    pub mode: NoteMode,
    /// `true` = tutorial folded away (button only).
    #[serde(default = "default_true")]
    pub fiche_pliee: bool,
    /// Opt-in finger drawing; one finger otherwise navigates and handles objects.
    #[serde(default)]
    pub finger_draw: bool,
    /// Column count used when `slot` was written. Missing field = the old 5-wide shelf.
    #[serde(default = "default_old_shelf_cols")]
    pub shelf_cols: u32,
}

fn default_true() -> bool {
    true
}

fn default_old_shelf_cols() -> u32 {
    5
}

impl Default for Index {
    fn default() -> Self {
        Self {
            seeded: false,
            notes: Vec::new(),
            trash: Vec::new(),
            dock: DockEdge::default(),
            mode: NoteMode::default(),
            fiche_pliee: true,
            finger_draw: false,
            shelf_cols: SHELF_COLS,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Library {
    pub root: PathBuf,
    pub index: Index,
}

impl Library {
    pub fn open() -> Self {
        let root = data_dir();
        let _ = fs::create_dir_all(root.join("notes"));
        let index_path = root.join("library.json");
        let index = fs::read_to_string(&index_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut lib = Self { root, index };
        lib.ensure_slots();
        lib.ensure_trash_slots();
        lib
    }

    pub fn save_index(&self) {
        let _ = fs::create_dir_all(&self.root);
        if let Ok(s) = serde_json::to_string_pretty(&self.index) {
            let _ = fs::write(self.root.join("library.json"), s);
        }
    }

    pub fn note_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("notes").join(id.to_string())
    }

    pub fn load_note(&self, id: Uuid) -> Option<Note> {
        let path = self.note_dir(id).join("note.json");
        let s = fs::read_to_string(path).ok()?;
        let mut note: Note = serde_json::from_str(&s).ok()?;
        note.normalize_page_grid();
        Some(note)
    }

    pub fn save_note(&mut self, note: &Note) {
        let dir = self.note_dir(note.id);
        let _ = fs::create_dir_all(dir.join("media"));
        if let Ok(s) = serde_json::to_string(&note) {
            let _ = fs::write(dir.join("note.json"), s);
        }
        if let Some(meta) = self.index.notes.iter_mut().find(|m| m.id == note.id) {
            meta.title = note.title.clone();
            meta.updated = note.updated;
            meta.pinned = note.pinned;
            meta.cover = note.cover;
            meta.emoji = note.emoji.clone();
        } else {
            let slot = self.first_free_slot();
            self.index.notes.insert(
                0,
                NoteMeta {
                    id: note.id,
                    title: note.title.clone(),
                    updated: note.updated,
                    pinned: note.pinned,
                    cover: note.cover,
                    emoji: note.emoji.clone(),
                    slot,
                },
            );
        }
        self.save_index();
    }

    fn first_free_slot(&self) -> u32 {
        let used: HashSet<u32> = self.index.notes.iter().map(|m| m.slot).collect();
        (0..SHELF_SLOTS)
            .find(|i| !used.contains(i))
            .unwrap_or(SHELF_SLOTS)
    }

    fn first_free_trash_slot(&self) -> u32 {
        let used: HashSet<u32> = self.index.trash.iter().map(|m| m.slot).collect();
        (0..TRASH_SLOTS)
            .find(|i| !used.contains(i))
            .unwrap_or(TRASH_SLOTS)
    }

    pub fn is_trashed(&self, id: Uuid) -> bool {
        self.index.trash.iter().any(|m| m.id == id)
    }

    fn ensure_slots(&mut self) {
        self.remap_shelf_cols();
        let mut seen = HashSet::new();
        let clash = self.index.notes.iter().any(|n| !seen.insert(n.slot));
        let all_zero = self.index.notes.len() > 1 && self.index.notes.iter().all(|n| n.slot == 0);
        if clash || all_zero {
            for (i, n) in self.index.notes.iter_mut().enumerate() {
                n.slot = i as u32;
            }
        }
        let used: HashSet<u32> = self
            .index
            .notes
            .iter()
            .filter(|n| n.slot < SHELF_SLOTS)
            .map(|n| n.slot)
            .collect();
        let mut free: Vec<u32> = (0..SHELF_SLOTS).filter(|i| !used.contains(i)).collect();
        free.reverse();
        let mut dirty = clash || all_zero;
        for n in self.index.notes.iter_mut() {
            if n.slot < SHELF_SLOTS {
                continue;
            }
            if let Some(slot) = free.pop() {
                n.slot = slot;
                dirty = true;
            }
        }
        if dirty {
            self.save_index();
        }
    }

    fn ensure_trash_slots(&mut self) {
        let mut seen = HashSet::new();
        let clash = self.index.trash.iter().any(|n| !seen.insert(n.slot));
        let mut dirty = false;
        if clash {
            for (i, n) in self.index.trash.iter_mut().enumerate() {
                n.slot = i as u32;
            }
            dirty = true;
        }
        let used: HashSet<u32> = self
            .index
            .trash
            .iter()
            .filter(|n| n.slot < TRASH_SLOTS)
            .map(|n| n.slot)
            .collect();
        let mut free: Vec<u32> = (0..TRASH_SLOTS).filter(|i| !used.contains(i)).collect();
        free.reverse();
        for n in self.index.trash.iter_mut() {
            if n.slot < TRASH_SLOTS {
                continue;
            }
            if let Some(slot) = free.pop() {
                n.slot = slot;
                dirty = true;
            }
        }
        if dirty {
            self.save_index();
        }
    }

    fn remap_shelf_cols(&mut self) {
        let old = self.index.shelf_cols.max(1);
        if old == SHELF_COLS {
            return;
        }
        let remap = |slot: u32| {
            let col = slot % old;
            let row = slot / old;
            row * SHELF_COLS + col
        };
        for n in &mut self.index.notes {
            n.slot = remap(n.slot);
        }
        for n in &mut self.index.trash {
            n.slot = remap(n.slot);
        }
        self.index.shelf_cols = SHELF_COLS;
        self.save_index();
    }

    /// Places notebooks on cells `dest`, `dest+1`, … (swap if occupied).
    pub fn place_at(&mut self, moving: &[Uuid], dest: u32) {
        if moving.is_empty() {
            return;
        }
        let dest = dest.min(SHELF_SLOTS.saturating_sub(1));
        let old: Vec<u32> = moving
            .iter()
            .filter_map(|id| {
                self.index
                    .notes
                    .iter()
                    .find(|n| n.id == *id)
                    .map(|n| n.slot)
            })
            .collect();
        for (k, id) in moving.iter().enumerate() {
            let target = dest.saturating_add(k as u32);
            if target >= SHELF_SLOTS {
                continue;
            }
            let prev = old.get(k).copied();
            if let Some(other) = self
                .index
                .notes
                .iter_mut()
                .find(|n| n.slot == target && !moving.contains(&n.id))
            {
                other.slot = prev.unwrap_or(target);
            }
            if let Some(n) = self.index.notes.iter_mut().find(|n| n.id == *id) {
                n.slot = target;
            }
        }
        self.save_index();
    }

    /// Places notebooks on bin cells `dest`, `dest+1`, … (swap if occupied).
    pub fn place_trash_at(&mut self, moving: &[Uuid], dest: u32) {
        if moving.is_empty() {
            return;
        }
        let dest = dest.min(TRASH_SLOTS.saturating_sub(1));
        let old: Vec<u32> = moving
            .iter()
            .filter_map(|id| {
                self.index
                    .trash
                    .iter()
                    .find(|n| n.id == *id)
                    .map(|n| n.slot)
            })
            .collect();
        for (k, id) in moving.iter().enumerate() {
            let target = dest.saturating_add(k as u32);
            if target >= TRASH_SLOTS {
                continue;
            }
            let prev = old.get(k).copied();
            if let Some(other) = self
                .index
                .trash
                .iter_mut()
                .find(|n| n.slot == target && !moving.contains(&n.id))
            {
                other.slot = prev.unwrap_or(target);
            }
            if let Some(n) = self.index.trash.iter_mut().find(|n| n.id == *id) {
                n.slot = target;
            }
        }
        self.save_index();
    }

    pub fn restore_at(&mut self, moving: &[Uuid], dest: u32) {
        for id in moving {
            self.restore_note(*id);
        }
        self.place_at(moving, dest);
    }

    pub fn trash_at(&mut self, moving: &[Uuid], dest: u32) {
        for id in moving {
            self.trash_note(*id);
        }
        self.place_trash_at(moving, dest);
    }

    pub fn bring_front(&mut self, id: Uuid) {
        if let Some(i) = self.index.notes.iter().position(|m| m.id == id) {
            if i == 0 {
                return;
            }
            let m = self.index.notes.remove(i);
            self.index.notes.insert(0, m);
            self.save_index();
        }
    }

    pub fn insert_new(&mut self, note: &Note) {
        self.save_note(note);
    }

    /// Moves the notebook into the trash (files kept).
    pub fn trash_note(&mut self, id: Uuid) {
        if let Some(i) = self.index.notes.iter().position(|m| m.id == id) {
            let mut meta = self.index.notes.remove(i);
            self.index.trash.retain(|m| m.id != id);
            meta.slot = self.first_free_trash_slot();
            self.index.trash.push(meta);
            self.save_index();
        }
    }

    pub fn restore_note(&mut self, id: Uuid) {
        if let Some(i) = self.index.trash.iter().position(|m| m.id == id) {
            let meta = self.index.trash.remove(i);
            self.index.notes.retain(|m| m.id != id);
            let used: HashSet<u32> = self.index.notes.iter().map(|m| m.slot).collect();
            let mut meta = meta;
            if meta.slot >= SHELF_SLOTS || used.contains(&meta.slot) {
                meta.slot = self.first_free_slot();
            }
            self.index.notes.push(meta);
            self.save_index();
        }
    }

    pub fn purge_trashed(&mut self, id: Uuid) {
        self.index.trash.retain(|m| m.id != id);
        let _ = fs::remove_dir_all(self.note_dir(id));
        self.save_index();
    }

    pub fn empty_trash(&mut self) {
        let ids: Vec<_> = self.index.trash.iter().map(|m| m.id).collect();
        self.index.trash.clear();
        for id in ids {
            let _ = fs::remove_dir_all(self.note_dir(id));
        }
        self.save_index();
    }

    pub fn write_media(&self, note_id: Uuid, bytes: &[u8]) -> Option<String> {
        let id = Uuid::new_v4();
        let name = format!("{id}.png");
        let dir = self.note_dir(note_id).join("media");
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join(&name), bytes).ok()?;
        Some(name)
    }

    pub fn media_path(&self, note_id: Uuid, file: &str) -> PathBuf {
        self.note_dir(note_id).join("media").join(file)
    }

    pub fn mark_seeded(&mut self) {
        self.index.seeded = true;
        self.save_index();
    }
}

pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("CAHIER_DATA") {
        return PathBuf::from(p);
    }
    directories::ProjectDirs::from("com", "clouddown", "omacourses")
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".").join("data"))
}

pub fn ensure_png(bytes: &[u8]) -> Option<Vec<u8>> {
    // Decode only a bounded source image. The saved canvas is capped further
    // below, but limiting the decoder itself keeps a malformed or enormous
    // upload from exhausting the application's memory before it is resized.
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let img = img.into_rgba8();
    let (mut w, mut h) = img.dimensions();
    let max = 2048u32;
    let rgba = if w > max || h > max {
        let s = (max as f32 / w.max(h) as f32).min(1.0);
        w = ((w as f32 * s) as u32).max(1);
        h = ((h as f32 * s) as u32).max(1);
        image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

pub fn image_size(path: &Path) -> Option<(f32, f32)> {
    let (w, h) = image::image_dimensions(path).ok()?;
    Some((w as f32, h as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remap_5_to_10_keeps_rows() {
        let old = 5u32;
        let remap = |slot: u32| slot % old + (slot / old) * SHELF_COLS;
        assert_eq!(remap(0), 0);
        assert_eq!(remap(4), 4);
        assert_eq!(remap(5), 10);
        assert_eq!(remap(9), 14);
        assert_eq!(remap(10), 20);
        assert_eq!(remap(14), 24);
    }

    #[test]
    fn imported_images_are_normalized_and_decoder_is_bounded() {
        let small = image::RgbaImage::from_pixel(3000, 2, image::Rgba([8, 16, 24, 255]));
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgba8(small)
            .write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Png)
            .unwrap();
        let normalized = ensure_png(&encoded).expect("normal-sized import accepted");
        let normalized = image::load_from_memory(&normalized).unwrap();
        assert_eq!((normalized.width(), normalized.height()), (2048, 1));

        // A tiny PNG file can claim an enormous width. Reject it before the
        // decoder tries to reserve an equally enormous pixel buffer.
        let too_wide = image::RgbaImage::from_pixel(8193, 1, image::Rgba([0, 0, 0, 0]));
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgba8(too_wide)
            .write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Png)
            .unwrap();
        assert!(ensure_png(&encoded).is_none());
    }
}
