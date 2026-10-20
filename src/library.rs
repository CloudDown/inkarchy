use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::document::Note;
use crate::ink::Tool;
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
    /// Tools hidden from the note dock and shown in its storage tray.
    #[serde(default)]
    pub parked_tools: Vec<Tool>,
    /// User-mixed opaque RGB inks, appended to the theme palette.
    #[serde(default)]
    pub custom_inks: Vec<[u8; 3]>,
    /// Optional exercise coaching; credentials are stored in the system keyring.
    #[serde(default)]
    pub ai: crate::ai::AiSettings,
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
            parked_tools: Vec::new(),
            custom_inks: Vec::new(),
            ai: crate::ai::AiSettings::default(),
        }
    }
}

/// A notebook or shelf file could not be written.
#[derive(Debug)]
pub enum SaveError {
    Io { path: PathBuf, source: io::Error },
    Encode(serde_json::Error),
}

impl SaveError {
    fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "could not write {}: {source}", path.display())
            }
            Self::Encode(err) => write!(f, "could not encode the notebook: {err}"),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Encode(err) => Some(err),
        }
    }
}

/// Write `bytes` by creating a temp file in the same directory, fsyncing it, then renaming.
/// A failed write leaves the previous file in place.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let tmp_path = parent.join(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    let write_tmp = (|| {
        let mut file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(err) = write_tmp {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Library {
    pub root: PathBuf,
    pub index: Index,
    pending_save_error: Option<String>,
}

impl Library {
    /// Empty library rooted at `root`, without reading or creating files.
    #[cfg(test)]
    pub fn fresh(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            index: Index::default(),
            pending_save_error: None,
        }
    }

    pub fn open() -> Self {
        let root = data_dir();
        let _ = fs::create_dir_all(root.join("notes"));
        let index_path = root.join("library.json");
        let index = fs::read_to_string(&index_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut lib = Self {
            root,
            index,
            pending_save_error: None,
        };
        lib.ensure_slots();
        lib.ensure_trash_slots();
        lib
    }

    /// Startup repair can fail before the window exists. The app shows this once.
    pub fn take_save_error(&mut self) -> Option<String> {
        self.pending_save_error.take()
    }

    pub(crate) fn remember_save_error(&mut self, result: Result<(), SaveError>) {
        if let Err(err) = result {
            self.pending_save_error = Some(err.to_string());
        }
    }

    pub fn save_index(&self) -> Result<(), SaveError> {
        fs::create_dir_all(&self.root).map_err(|source| SaveError::io(&self.root, source))?;
        let path = self.root.join("library.json");
        let bytes = serde_json::to_vec_pretty(&self.index).map_err(SaveError::Encode)?;
        atomic_write(&path, &bytes).map_err(|source| SaveError::io(path, source))
    }

    pub fn note_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("notes").join(id.to_string())
    }

    pub fn load_note(&self, id: Uuid) -> Option<Note> {
        let path = self.note_dir(id).join("note.json");
        let s = fs::read_to_string(path).ok()?;
        let mut note: Note = serde_json::from_str(&s).ok()?;
        note.normalize_page_grid();
        note.sync_pdf_sheet_size();
        Some(note)
    }

    pub fn save_note(&mut self, note: &Note) -> Result<(), SaveError> {
        let dir = self.note_dir(note.id);
        let media = dir.join("media");
        fs::create_dir_all(&media).map_err(|source| SaveError::io(&media, source))?;
        let path = dir.join("note.json");
        let bytes = serde_json::to_vec(note).map_err(SaveError::Encode)?;
        atomic_write(&path, &bytes).map_err(|source| SaveError::io(&path, source))?;
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
        self.save_index()
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
            let saved = self.save_index();
            self.remember_save_error(saved);
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
            let saved = self.save_index();
            self.remember_save_error(saved);
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
        let saved = self.save_index();
        self.remember_save_error(saved);
    }

    /// Places notebooks on cells `dest`, `dest+1`, … (swap if occupied).
    pub fn place_at(&mut self, moving: &[Uuid], dest: u32) -> Result<(), SaveError> {
        if moving.is_empty() {
            return Ok(());
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
        self.save_index()
    }

    /// Places notebooks on bin cells `dest`, `dest+1`, … (swap if occupied).
    pub fn place_trash_at(&mut self, moving: &[Uuid], dest: u32) -> Result<(), SaveError> {
        if moving.is_empty() {
            return Ok(());
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
        self.save_index()
    }

    pub fn restore_at(&mut self, moving: &[Uuid], dest: u32) -> Result<(), SaveError> {
        for id in moving {
            self.restore_note(*id)?;
        }
        self.place_at(moving, dest)
    }

    pub fn trash_at(&mut self, moving: &[Uuid], dest: u32) -> Result<(), SaveError> {
        for id in moving {
            self.trash_note(*id)?;
        }
        self.place_trash_at(moving, dest)
    }

    pub fn insert_new(&mut self, note: &Note) -> Result<(), SaveError> {
        self.save_note(note)
    }

    /// Moves the notebook into the trash (files kept).
    pub fn trash_note(&mut self, id: Uuid) -> Result<(), SaveError> {
        if let Some(i) = self.index.notes.iter().position(|m| m.id == id) {
            let mut meta = self.index.notes.remove(i);
            self.index.trash.retain(|m| m.id != id);
            meta.slot = self.first_free_trash_slot();
            self.index.trash.push(meta);
            self.save_index()?;
        }
        Ok(())
    }

    pub fn restore_note(&mut self, id: Uuid) -> Result<(), SaveError> {
        if let Some(i) = self.index.trash.iter().position(|m| m.id == id) {
            let meta = self.index.trash.remove(i);
            self.index.notes.retain(|m| m.id != id);
            let used: HashSet<u32> = self.index.notes.iter().map(|m| m.slot).collect();
            let mut meta = meta;
            if meta.slot >= SHELF_SLOTS || used.contains(&meta.slot) {
                meta.slot = self.first_free_slot();
            }
            self.index.notes.push(meta);
            self.save_index()?;
        }
        Ok(())
    }

    pub fn purge_trashed(&mut self, id: Uuid) -> Result<(), SaveError> {
        self.index.trash.retain(|m| m.id != id);
        let dir = self.note_dir(id);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|source| SaveError::io(&dir, source))?;
        }
        self.save_index()
    }

    pub fn empty_trash(&mut self) -> Result<(), SaveError> {
        let ids: Vec<_> = self.index.trash.iter().map(|m| m.id).collect();
        self.index.trash.clear();
        for id in ids {
            let dir = self.note_dir(id);
            if dir.exists() {
                fs::remove_dir_all(&dir).map_err(|source| SaveError::io(&dir, source))?;
            }
        }
        self.save_index()
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

    pub fn mark_seeded(&mut self) -> Result<(), SaveError> {
        self.index.seeded = true;
        self.save_index()
    }
}

pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("INKARCHY_DATA") {
        return PathBuf::from(p);
    }
    // Deprecated alias kept so existing wrappers keep working.
    if let Some(p) = std::env::var_os("CAHIER_DATA") {
        return PathBuf::from(p);
    }
    let dest = project_data_dir("inkarchy");
    let src = project_data_dir("omacourses");
    migrate_legacy_data_dir(&src, &dest);
    dest
}

fn project_data_dir(application: &str) -> PathBuf {
    directories::ProjectDirs::from("com", "clouddown", application)
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".").join("data"))
}

/// Move `~/.local/share/omacourses` to `~/.local/share/inkarchy` once.
/// Rename first; copy then remove the source if rename fails (e.g. cross-device).
fn migrate_legacy_data_dir(src: &Path, dest: &Path) {
    if dest.exists() || !src.exists() || src == dest {
        return;
    }
    if let Some(parent) = dest.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            eprintln!(
                "inkarchy: failed to migrate data from {} to {}: {err}",
                src.display(),
                dest.display()
            );
            return;
        }
    }
    match fs::rename(src, dest) {
        Ok(()) => {
            eprintln!(
                "inkarchy: migrated data from {} to {}",
                src.display(),
                dest.display()
            );
        }
        Err(rename_err) => {
            if dest.exists() {
                return;
            }
            match copy_dir_recursive(src, dest) {
                Ok(()) => {
                    eprintln!(
                        "inkarchy: copied data from {} to {} (rename failed: {rename_err})",
                        src.display(),
                        dest.display()
                    );
                    if let Err(err) = fs::remove_dir_all(src) {
                        eprintln!(
                            "inkarchy: copied data to {}, but could not remove {}: {err}",
                            dest.display(),
                            src.display()
                        );
                    }
                }
                Err(copy_err) => {
                    let _ = fs::remove_dir_all(dest);
                    eprintln!(
                        "inkarchy: failed to migrate data from {} to {}: rename: {rename_err}; copy: {copy_err}",
                        src.display(),
                        dest.display()
                    );
                }
            }
        }
    }
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
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
    use std::os::unix::fs::PermissionsExt;

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
    fn parked_tools_default_for_old_indexes_and_round_trip() {
        let old: Index = serde_json::from_str(r#"{"seeded":false,"notes":[]}"#)
            .expect("load an index written before tool storage existed");
        assert!(old.parked_tools.is_empty());
        assert!(old.custom_inks.is_empty());

        let index = Index {
            parked_tools: vec![Tool::Text, Tool::EraserStroke],
            custom_inks: vec![[25, 80, 160]],
            ..Index::default()
        };
        let encoded = serde_json::to_vec(&index).expect("encode index");
        let decoded: Index = serde_json::from_slice(&encoded).expect("decode index");
        assert_eq!(decoded.parked_tools, index.parked_tools);
        assert_eq!(decoded.custom_inks, index.custom_inks);
    }

    #[test]
    fn atomic_save_roundtrip_replaces_the_file_and_leaves_no_temp() {
        let root = std::env::temp_dir().join(format!("inkarchy-atomic-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        let mut lib = Library::fresh(&root);
        let note = crate::document::Note::blank("Atomic", 1);
        lib.save_note(&note).expect("save note");
        let loaded = lib.load_note(note.id).expect("load note");
        assert_eq!(loaded.title, "Atomic");
        assert!(root.join("library.json").is_file());
        assert!(lib.note_dir(note.id).join("note.json").is_file());

        let mut rewritten = loaded;
        rewritten.title = "Rewritten".into();
        lib.save_note(&rewritten).expect("rewrite note");
        let text = fs::read_to_string(lib.note_dir(note.id).join("note.json")).expect("read note");
        assert!(text.contains("Rewritten"));
        assert!(!text.contains("Atomic"));
        assert!(
            !dir_has_tmp(&root),
            "temp file left behind after a successful save"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn atomic_write_failure_keeps_the_previous_file() {
        let root = std::env::temp_dir().join(format!("inkarchy-atomic-ro-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create root");
        let path = root.join("library.json");
        atomic_write(&path, br#"{"kept":true}"#).expect("first write");

        let original = fs::metadata(&root).expect("metadata").permissions().mode();
        let mut locked = fs::metadata(&root).expect("metadata").permissions();
        locked.set_mode(original & !0o222);
        fs::set_permissions(&root, locked).expect("lock directory");
        let _unlock = UnlockDir(&root, original);

        let err =
            atomic_write(&path, br#"{"kept":false}"#).expect_err("write into a locked directory");
        assert!(!err.to_string().is_empty());
        assert_eq!(fs::read(&path).expect("read original"), br#"{"kept":true}"#);
        drop(_unlock);
        assert!(!dir_has_tmp(&root));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn save_index_reports_a_directory_that_cannot_be_created() {
        let root = std::env::temp_dir().join(format!("inkarchy-atomic-file-{}", Uuid::new_v4()));
        let _ = fs::remove_file(&root);
        fs::write(&root, b"not a directory").expect("blocker file");
        let lib = Library::fresh(&root);
        let err = lib.save_index().expect_err("root is a file");
        let message = err.to_string();
        assert!(message.contains("could not write"), "{message}");
        assert_eq!(fs::read(&root).expect("blocker intact"), b"not a directory");
        let _ = fs::remove_file(&root);
    }

    fn dir_has_tmp(root: &Path) -> bool {
        fn walk(dir: &Path, found: &mut bool) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for ent in entries.filter_map(Result::ok) {
                let path = ent.path();
                if ent.file_name().to_string_lossy().contains(".tmp") {
                    *found = true;
                }
                if path.is_dir() {
                    walk(&path, found);
                }
            }
        }
        let mut found = false;
        walk(root, &mut found);
        found
    }

    struct UnlockDir<'a>(&'a Path, u32);

    impl Drop for UnlockDir<'_> {
        fn drop(&mut self) {
            if let Ok(meta) = fs::metadata(self.0) {
                let mut perms = meta.permissions();
                perms.set_mode(self.1);
                let _ = fs::set_permissions(self.0, perms);
            }
        }
    }

    #[test]
    fn migrates_legacy_data_dir_when_the_new_dir_is_absent() {
        let root = std::env::temp_dir().join(format!("inkarchy-migrate-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        let src = root.join("omacourses");
        let dest = root.join("inkarchy");
        fs::create_dir_all(src.join("notes")).expect("legacy notes dir");
        fs::write(src.join("library.json"), br#"{"seeded":true}"#).expect("legacy index");
        fs::write(src.join("notes").join("keep.txt"), b"ok").expect("legacy note file");

        migrate_legacy_data_dir(&src, &dest);

        assert!(dest.join("library.json").is_file());
        assert_eq!(
            fs::read(dest.join("notes").join("keep.txt")).expect("migrated file"),
            b"ok"
        );
        assert!(
            !src.exists(),
            "legacy directory should be gone after rename"
        );

        fs::write(dest.join("library.json"), br#"{"seeded":true,"kept":true}"#).expect("mark dest");
        let leftover = root.join("omacourses-again");
        fs::create_dir_all(&leftover).expect("new leftover");
        fs::write(leftover.join("library.json"), br#"{"seeded":false}"#).expect("leftover index");
        migrate_legacy_data_dir(&leftover, &dest);
        assert_eq!(
            fs::read(dest.join("library.json")).expect("dest unchanged"),
            br#"{"seeded":true,"kept":true}"#
        );
        assert!(leftover.join("library.json").is_file());

        let copy_src = root.join("omacourses-copy");
        let copy_dest = root.join("inkarchy-copy");
        fs::create_dir_all(copy_src.join("notes")).expect("copy src");
        fs::write(copy_src.join("library.json"), b"copied").expect("copy index");
        copy_dir_recursive(&copy_src, &copy_dest).expect("copy fallback");
        assert_eq!(
            fs::read(copy_dest.join("library.json")).expect("copied index"),
            b"copied"
        );
        assert!(copy_src.join("library.json").is_file());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn migrates_legacy_data_dir_across_devices_by_copy() {
        let shm = Path::new("/dev/shm");
        if !shm.is_dir() {
            return;
        }
        let src_root = std::env::temp_dir().join(format!("inkarchy-exdev-src-{}", Uuid::new_v4()));
        let dest_root = shm.join(format!("inkarchy-exdev-dst-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&src_root);
        let _ = fs::remove_dir_all(&dest_root);
        let src = src_root.join("omacourses");
        let dest = dest_root.join("inkarchy");
        fs::create_dir_all(src.join("notes")).expect("legacy notes dir");
        fs::write(src.join("library.json"), b"cross").expect("legacy index");
        fs::write(src.join("notes").join("keep.txt"), b"ok").expect("legacy note file");

        migrate_legacy_data_dir(&src, &dest);

        assert_eq!(
            fs::read(dest.join("library.json")).expect("migrated index"),
            b"cross"
        );
        assert_eq!(
            fs::read(dest.join("notes").join("keep.txt")).expect("migrated file"),
            b"ok"
        );
        assert!(!src.exists());
        let _ = fs::remove_dir_all(&src_root);
        let _ = fs::remove_dir_all(&dest_root);
    }

    #[test]
    fn imported_images_are_normalized_and_decoder_is_bounded() {
        let small = image::RgbaImage::from_pixel(3000, 2, image::Rgba([8, 16, 24, 255]));
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgba8(small)
            .write_to(
                &mut std::io::Cursor::new(&mut encoded),
                image::ImageFormat::Png,
            )
            .unwrap();
        let normalized = ensure_png(&encoded).expect("normal-sized import accepted");
        let normalized = image::load_from_memory(&normalized).unwrap();
        assert_eq!((normalized.width(), normalized.height()), (2048, 1));

        // A tiny PNG file can claim an enormous width. Reject it before the
        // decoder tries to reserve an equally enormous pixel buffer.
        let too_wide = image::RgbaImage::from_pixel(8193, 1, image::Rgba([0, 0, 0, 0]));
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgba8(too_wide)
            .write_to(
                &mut std::io::Cursor::new(&mut encoded),
                image::ImageFormat::Png,
            )
            .unwrap();
        assert!(ensure_png(&encoded).is_none());
    }
}
