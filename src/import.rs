//! Open a PDF, image, or small text file as a notebook.

use std::fs;
use std::path::Path;

use crate::document::{Note, Page, PageBackground, PaperKind, SheetJoin, TextBox, PAGE_W};
use crate::library::{ensure_png, Library};
use crate::pdf::{self, logical_size};

const TEXT_LIMIT: usize = 64 * 1024;

#[derive(Debug)]
pub enum ImportError {
    Message(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(msg) => f.write_str(msg),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Pdf,
    RasterImage,
    Svg,
    Video,
    Text,
    Unsupported,
}

pub fn classify(path: &Path) -> FileKind {
    match extension(path).as_str() {
        "pdf" => FileKind::Pdf,
        "png" | "jpg" | "jpeg" | "webp" => FileKind::RasterImage,
        "svg" => FileKind::Svg,
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "ogv" => FileKind::Video,
        "txt" | "md" | "markdown" => FileKind::Text,
        _ => sniff(path),
    }
}

pub fn is_placeable_image(path: &Path) -> bool {
    matches!(classify(path), FileKind::RasterImage | FileKind::Svg)
}

pub fn is_placeable_video(path: &Path) -> bool {
    matches!(classify(path), FileKind::Video)
}

/// Images and videos both land directly on the open page.
pub fn is_placeable_media(path: &Path) -> bool {
    is_placeable_image(path) || is_placeable_video(path)
}

/// What the media tool should place on the page.
pub enum PlacedMedia {
    Image(Vec<u8>),
    Video {
        ext: String,
        meta: crate::video::Meta,
    },
}

/// Decode images now; videos are only probed so a large file is copied, not
/// loaded into the event loop.
pub fn load_placed_media(path: &Path) -> Result<PlacedMedia, ImportError> {
    if is_placeable_video(path) {
        let ext = crate::video::extension(path)
            .ok_or_else(|| ImportError::Message(format!("Can't place {}", display_name(path))))?;
        let meta = crate::video::probe(path).ok_or_else(|| {
            ImportError::Message(format!(
                "Could not read {} (ffmpeg and ffprobe are needed for videos)",
                display_name(path)
            ))
        })?;
        return Ok(PlacedMedia::Video { ext, meta });
    }
    load_placed_image(path).map(PlacedMedia::Image)
}

/// PNG bytes for an image placed on the current page.
pub fn load_placed_image(path: &Path) -> Result<Vec<u8>, ImportError> {
    let bytes = fs::read(path)
        .map_err(|err| ImportError::Message(format!("Could not read {}: {err}", path.display())))?;
    match classify(path) {
        FileKind::Svg => svg_to_png(&bytes)
            .ok_or_else(|| ImportError::Message(format!("Could not read {}", display_name(path)))),
        FileKind::RasterImage => ensure_png(&bytes)
            .ok_or_else(|| ImportError::Message(format!("Could not read {}", display_name(path)))),
        _ => Err(ImportError::Message(format!(
            "Can't place {}",
            display_name(path)
        ))),
    }
}

/// A new notebook. The original PDF is copied into the note folder.
pub fn import_new_note(lib: &mut Library, path: &Path) -> Result<Note, ImportError> {
    if !path.is_file() {
        return Err(ImportError::Message(format!(
            "{} is not a file",
            path.display()
        )));
    }
    match classify(path) {
        FileKind::Pdf => import_pdf(lib, path),
        FileKind::RasterImage | FileKind::Svg => import_image_note(lib, path),
        FileKind::Video => Err(ImportError::Message(format!(
            "Place {} on an open page",
            display_name(path)
        ))),
        FileKind::Text => import_text(lib, path),
        FileKind::Unsupported => Err(ImportError::Message(format!(
            "Can't open {}",
            display_name(path)
        ))),
    }
}

fn import_pdf(lib: &mut Library, path: &Path) -> Result<Note, ImportError> {
    let bytes = fs::read(path)
        .map_err(|err| ImportError::Message(format!("Could not read {}: {err}", path.display())))?;
    let specs = pdf::inspect_pdf(&bytes).map_err(ImportError::Message)?;
    let mut note = Note::blank(title_from(path), cover_of(lib));
    note.paper = PaperKind::Blank;
    if specs.len() > 1 {
        note.sheet_join = SheetJoin::Separate;
    }
    let (page_w, page_h) = specs.iter().fold((1.0f32, 1.0f32), |(aw, ah), spec| {
        let (w, h) = logical_size(spec.width_pt, spec.height_pt);
        (aw.max(w), ah.max(h))
    });
    note.page_w = page_w;
    note.page_h = page_h;
    note.pages = specs
        .iter()
        .enumerate()
        .map(|(i, spec)| Page {
            row: i as i32,
            background: Some(PageBackground {
                file: "source.pdf".into(),
                pdf_page: Some(i as u32),
                source_size: Some([spec.width_pt, spec.height_pt]),
            }),
            ..Page::at(0, i as i32)
        })
        .collect();
    let dir = lib.note_dir(note.id);
    fs::create_dir_all(dir.join("media"))
        .map_err(|err| ImportError::Message(format!("Could not create the notebook: {err}")))?;
    fs::write(dir.join("source.pdf"), &bytes)
        .map_err(|err| ImportError::Message(format!("Could not keep the PDF: {err}")))?;
    lib.save_note(&note)
        .map_err(|err| ImportError::Message(err.to_string()))?;
    Ok(note)
}

fn import_image_note(lib: &mut Library, path: &Path) -> Result<Note, ImportError> {
    let png = load_placed_image(path)?;
    let (px_w, px_h) = png_size(&png).unwrap_or((800.0, 600.0));
    let (page_w, page_h) = image_page_size(px_w, px_h);
    let mut note = Note::blank(title_from(path), cover_of(lib));
    note.paper = PaperKind::Blank;
    note.page_w = page_w;
    note.page_h = page_h;
    let dir = lib.note_dir(note.id);
    fs::create_dir_all(dir.join("media"))
        .map_err(|err| ImportError::Message(format!("Could not create the notebook: {err}")))?;
    let Some(name) = lib.write_media(note.id, &png) else {
        return Err(ImportError::Message("Could not store the image.".into()));
    };
    note.pages[0].background = Some(PageBackground {
        file: format!("media/{name}"),
        pdf_page: None,
        source_size: Some([px_w, px_h]),
    });
    lib.save_note(&note)
        .map_err(|err| ImportError::Message(err.to_string()))?;
    Ok(note)
}

fn import_text(lib: &mut Library, path: &Path) -> Result<Note, ImportError> {
    let bytes = fs::read(path)
        .map_err(|err| ImportError::Message(format!("Could not read {}: {err}", path.display())))?;
    if bytes.len() > TEXT_LIMIT || bytes.contains(&0) {
        return Err(ImportError::Message(format!(
            "Skipped {}",
            display_name(path)
        )));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| ImportError::Message(format!("Skipped {}", display_name(path))))?;
    let mut note = Note::blank(title_from(path), cover_of(lib));
    note.paper = PaperKind::Lined;
    note.pages[0].texts.push(TextBox {
        id: uuid::Uuid::new_v4(),
        pos: [56.0, 48.0],
        size: [PAGE_W - 112.0, note.page_h - 96.0],
        text,
        size_pt: 16.0,
        color: [36, 32, 28, 255],
        sticky: false,
    });
    lib.save_note(&note)
        .map_err(|err| ImportError::Message(err.to_string()))?;
    Ok(note)
}

pub fn svg_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let mut w = size.width().ceil().max(1.0);
    let mut h = size.height().ceil().max(1.0);
    let mut transform = tiny_skia::Transform::default();
    const MAX: f32 = 2048.0;
    if w > MAX || h > MAX {
        let s = MAX / w.max(h);
        w = (w * s).max(1.0);
        h = (h * s).max(1.0);
        transform = tiny_skia::Transform::from_scale(s, s);
    }
    let mut pixmap = tiny_skia::Pixmap::new(w as u32, h as u32)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

fn image_page_size(px_w: f32, px_h: f32) -> (f32, f32) {
    let long = px_w.max(px_h).max(1.0);
    let s = PAGE_W / long;
    ((px_w * s).max(32.0), (px_h * s).max(32.0))
}

fn png_size(bytes: &[u8]) -> Option<(f32, f32)> {
    let img = image::load_from_memory(bytes).ok()?;
    Some((img.width() as f32, img.height() as f32))
}

fn cover_of(lib: &Library) -> u8 {
    (lib.index.notes.len() as u8).wrapping_add(3)
}

fn title_from(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Untitled")
        .to_string()
}

pub fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string()
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn sniff(path: &Path) -> FileKind {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return FileKind::Unsupported,
    };
    let mut head = [0u8; 16];
    let n = std::io::Read::read(&mut file, &mut head).unwrap_or(0);
    let head = &head[..n];
    if head.starts_with(b"%PDF") {
        FileKind::Pdf
    } else if head.starts_with(b"\x89PNG")
        || head.starts_with(&[0xff, 0xd8])
        || (head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP")
    {
        FileKind::RasterImage
    } else if (head.len() >= 12 && &head[4..8] == b"ftyp")
        || head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3])
        || (head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"AVI ")
    {
        FileKind::Video
    } else if head.starts_with(b"<svg") || head.starts_with(b"<?xml") {
        FileKind::Svg
    } else {
        FileKind::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::{marked_pdf, MarkedPage};
    use std::path::PathBuf;

    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn lib() -> (TempDir, Library) {
        let dir = std::env::temp_dir().join(format!("inkarchy-import-{}", uuid::Uuid::new_v4()));
        let _ = fs::remove_dir_all(&dir);
        (TempDir(dir.clone()), Library::fresh(dir))
    }

    #[test]
    fn pdf_import_copies_the_file_and_makes_one_page_each() {
        let (_tmp, mut lib) = lib();
        let bytes = marked_pdf(&[
            MarkedPage {
                width: 200.0,
                height: 100.0,
                rotate: 0,
                marker: "P1",
            },
            MarkedPage {
                width: 200.0,
                height: 100.0,
                rotate: 0,
                marker: "P2",
            },
        ]);
        let src = lib.root.join("lecture.pdf");
        fs::create_dir_all(&lib.root).unwrap();
        fs::write(&src, &bytes).unwrap();
        let note = import_new_note(&mut lib, &src).expect("import pdf");
        assert_eq!(note.pages.len(), 2);
        assert_eq!(note.sheet_join, SheetJoin::Separate);
        assert_eq!(note.pages[0].background.as_ref().unwrap().pdf_page, Some(0));
        assert_eq!(note.pages[1].background.as_ref().unwrap().pdf_page, Some(1));
        assert!(note.pages[0]
            .background
            .as_ref()
            .unwrap()
            .source_size
            .is_some());
        let copied = fs::read(lib.note_dir(note.id).join("source.pdf")).unwrap();
        assert_eq!(copied, bytes);
        let loaded = lib.load_note(note.id).unwrap();
        assert_eq!(loaded.pages.len(), 2);
        assert_eq!(loaded.title, "lecture");
        let (w, h) = logical_size(200.0, 100.0);
        assert!((note.page_w - w).abs() < 0.5);
        assert!((note.page_h - h).abs() < 0.5);
        assert!(note.page_w > note.page_h);
    }

    #[test]
    fn pdf_import_uses_the_largest_page() {
        let (_tmp, mut lib) = lib();
        let bytes = marked_pdf(&[
            MarkedPage {
                width: 200.0,
                height: 100.0,
                rotate: 0,
                marker: "P1",
            },
            MarkedPage {
                width: 100.0,
                height: 400.0,
                rotate: 0,
                marker: "P2",
            },
        ]);
        let src = lib.root.join("mix.pdf");
        fs::create_dir_all(&lib.root).unwrap();
        fs::write(&src, &bytes).unwrap();
        let note = import_new_note(&mut lib, &src).expect("import pdf");
        let (w1, h1) = logical_size(200.0, 100.0);
        let (w2, h2) = logical_size(100.0, 400.0);
        assert!((note.page_w - w1.max(w2)).abs() < 0.5);
        assert!((note.page_h - h1.max(h2)).abs() < 0.5);
    }

    #[test]
    fn png_and_svg_become_page_backgrounds() {
        let (_tmp, mut lib) = lib();
        fs::create_dir_all(&lib.root).unwrap();
        let png_path = lib.root.join("dot.png");
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            4,
            2,
            image::Rgba([10, 20, 30, 255]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
        fs::write(&png_path, &png).unwrap();
        let note = import_new_note(&mut lib, &png_path).unwrap();
        let bg = note.pages[0].background.as_ref().unwrap();
        assert!(bg.pdf_page.is_none());
        assert!(lib.note_dir(note.id).join(&bg.file).is_file());
        assert!((note.page_w / note.page_h - 2.0).abs() < 0.05);

        let svg_path = lib.root.join("box.svg");
        fs::write(
            &svg_path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="red"/></svg>"#,
        )
        .unwrap();
        let svg_note = import_new_note(&mut lib, &svg_path).unwrap();
        let svg_bg = svg_note.pages[0].background.as_ref().unwrap();
        let stored = image::open(lib.note_dir(svg_note.id).join(&svg_bg.file)).unwrap();
        assert!(stored.width() >= 10 && stored.height() >= 5);
    }

    #[test]
    fn small_text_is_a_text_block_and_a_large_file_is_skipped() {
        let (_tmp, mut lib) = lib();
        fs::create_dir_all(&lib.root).unwrap();
        let path = lib.root.join("notes.md");
        fs::write(&path, "# Hello\n\nA line.").unwrap();
        let note = import_new_note(&mut lib, &path).unwrap();
        assert!(note.pages[0].texts[0].text.contains("Hello"));
        assert_eq!(note.paper, PaperKind::Lined);

        let big = lib.root.join("huge.txt");
        fs::write(&big, vec![b'a'; TEXT_LIMIT + 8]).unwrap();
        let err = import_new_note(&mut lib, &big).unwrap_err();
        assert!(err.to_string().starts_with("Skipped"));
    }

    #[test]
    fn classify_uses_the_extension_and_pdf_magic() {
        let dir = std::env::temp_dir().join(format!("inkarchy-kind-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("noext");
        fs::write(&pdf, b"%PDF-1.4\n").unwrap();
        assert_eq!(classify(&pdf), FileKind::Pdf);
        assert_eq!(classify(Path::new("a.WEBP")), FileKind::RasterImage);
        assert_eq!(classify(Path::new("a.svg")), FileKind::Svg);
        assert_eq!(classify(Path::new("a.MP4")), FileKind::Video);
        assert_eq!(classify(Path::new("a.mkv")), FileKind::Video);
        assert_eq!(classify(Path::new("a.md")), FileKind::Text);
        assert_eq!(classify(Path::new("a.zip")), FileKind::Unsupported);
        let _ = fs::remove_dir_all(&dir);
    }
}
