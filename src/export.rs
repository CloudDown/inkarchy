use std::io::Write;

use crate::document::{Note, PageBackground, PaperKind, CORNELL_CUE, CORNELL_SUMMARY};
use crate::ink::{ribbon_outline, InkStroke};
use crate::look::Look;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use fontdue::{Font, FontSettings};
use tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Pixmap, Stroke as SkStroke, Transform};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RasterMode {
    /// Paper, background, and ink.
    Full,
    /// Ink, text, and placed images on a transparent page.
    Annotations,
}

pub fn raster_page(
    note: &Note,
    page_i: usize,
    look: &Look,
    scale: f32,
    media: &MediaLoader,
    note_dir: &std::path::Path,
    mode: RasterMode,
) -> Option<Pixmap> {
    let page = note.pages.get(page_i)?;
    let (page_w, page_h) = note.page_size();
    let w = (page_w * scale).round().max(1.0) as u32;
    let h = (page_h * scale).round().max(1.0) as u32;
    let mut pm = Pixmap::new(w, h)?;
    let t = Transform::from_scale(scale, scale);
    if mode == RasterMode::Full {
        let paper = if note.paper == PaperKind::Slate {
            rgb(look.desk_deep)
        } else {
            rgb(look.paper)
        };
        pm.fill(paper);
        let drew_bg = page
            .background
            .as_ref()
            .is_some_and(|bg| draw_background(&mut pm, note, bg, scale, note_dir));
        if !drew_bg {
            draw_template(&mut pm, note.paper, look, t, page_w, page_h);
        }
    }
    for im in &page.images {
        if let Some((iw, ih, rgba)) = media.rgba(&im.file) {
            blit(
                &mut pm,
                im.pos[0] * scale,
                im.pos[1] * scale,
                im.size[0] * scale,
                im.size[1] * scale,
                iw,
                ih,
                &rgba,
            );
        }
    }
    for s in &page.strokes {
        fill_stroke(&mut pm, s, t);
    }
    if let Some(font) = serif_font() {
        let ink = if note.paper == PaperKind::Slate {
            rgb(look.fg)
        } else {
            rgb(look.ink)
        };
        for tx in &page.texts {
            if tx.sticky {
                fill_rect(
                    &mut pm,
                    tx.pos[0] * scale,
                    tx.pos[1] * scale,
                    tx.size[0] * scale,
                    tx.size[1] * scale,
                    tx.color32(),
                    t,
                );
            }
            draw_text(
                &mut pm,
                &font,
                &tx.text,
                tx.pos[0] * scale,
                tx.pos[1] * scale,
                tx.size[0] * scale,
                tx.size_pt * scale,
                if tx.sticky { rgb(look.ink) } else { ink },
            );
        }
    }
    Some(pm)
}

pub fn pixmap_png(pm: &Pixmap) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(pm.width(), pm.height(), pm.data().to_vec())?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

pub fn pages_pdf(
    note: &Note,
    look: &Look,
    media: &MediaLoader,
    note_dir: &std::path::Path,
) -> Option<Vec<u8>> {
    if note_has_pdf_background(note) {
        if let Some(bytes) = stamp_original_pdf(note, look, media, note_dir) {
            return Some(bytes);
        }
    }
    let mut images = Vec::new();
    for i in 0..note.pages.len() {
        let pm = raster_page(note, i, look, 1.6, media, note_dir, RasterMode::Full)?;
        images.push((pm.width(), pm.height(), rgb_from_rgba(pm.data())));
    }
    Some(simple_pdf(&images))
}

fn note_has_pdf_background(note: &Note) -> bool {
    note.pages.iter().any(|page| {
        page.background
            .as_ref()
            .is_some_and(|bg| bg.pdf_page.is_some())
    })
}

fn draw_background(
    pm: &mut Pixmap,
    note: &Note,
    bg: &PageBackground,
    scale: f32,
    note_dir: &std::path::Path,
) -> bool {
    let (page_w, page_h) = note.page_size();
    let [x, y, w, h] = bg.fit(page_w, page_h);
    let loaded = if let Some(pdf_page) = bg.pdf_page {
        let pt = bg.source_size.unwrap_or([page_w, page_h]);
        let hayro_scale = ((w * scale) / pt[0].max(1.0)).clamp(0.25, 4.0);
        crate::pdf::render_pdf_file(&note_dir.join(&bg.file), pdf_page, hayro_scale)
            .ok()
            .map(|rendered| (rendered.width, rendered.height, rendered.rgba))
    } else {
        image::open(note_dir.join(&bg.file)).ok().map(|img| {
            let img = img.into_rgba8();
            let (iw, ih) = img.dimensions();
            (iw, ih, img.into_raw())
        })
    };
    let Some((iw, ih, rgba)) = loaded else {
        return false;
    };
    blit(
        pm,
        x * scale,
        y * scale,
        w * scale,
        h * scale,
        iw,
        ih,
        &rgba,
    );
    true
}

pub struct MediaLoader {
    pub root: std::path::PathBuf,
}

impl MediaLoader {
    fn rgba(&self, file: &str) -> Option<(u32, u32, Vec<u8>)> {
        let img = image::open(self.root.join(file)).ok()?.into_rgba8();
        let (w, h) = img.dimensions();
        Some((w, h, img.into_raw()))
    }
}

fn fill_stroke(pm: &mut Pixmap, stroke: &InkStroke, t: Transform) {
    let outline = ribbon_outline(&stroke.points, stroke.width, stroke.nib);
    let Some(path) = poly_path(&outline) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        stroke.color[0],
        stroke.color[1],
        stroke.color[2],
        stroke.color[3],
    );
    paint.anti_alias = true;
    pm.fill_path(&path, &paint, FillRule::Winding, t, None);
}

fn poly_path(pts: &[egui::Pos2]) -> Option<Path> {
    if pts.len() < 3 {
        return None;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].x, pts[0].y);
    for p in pts.iter().skip(1) {
        pb.line_to(p.x, p.y);
    }
    pb.close();
    pb.finish()
}

fn draw_template(
    pm: &mut Pixmap,
    kind: PaperKind,
    look: &Look,
    t: Transform,
    page_w: f32,
    page_h: f32,
) {
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    let mut stroke = SkStroke::default();
    match kind {
        PaperKind::Blank | PaperKind::Slate => {}
        PaperKind::Lined => {
            paint.set_color(rgb(look.paper_rule));
            stroke.width = 0.8;
            let mut y = 88.0;
            while y < page_h - 24.0 {
                line(pm, 56.0, y, page_w - 24.0, y, &paint, &stroke, t);
                y += 28.0;
            }
            let red = look.rust();
            let paper = look.paper;
            paint.set_color(Color::from_rgba8(
                (paper.r() as f32 * 0.62 + red.r() as f32 * 0.38) as u8,
                (paper.g() as f32 * 0.62 + red.g() as f32 * 0.38) as u8,
                (paper.b() as f32 * 0.62 + red.b() as f32 * 0.38) as u8,
                255,
            ));
            line(pm, 64.0, 24.0, 64.0, page_h - 24.0, &paint, &stroke, t);
        }
        PaperKind::Cornell => {
            paint.set_color(rgb(look.paper_rule_strong));
            stroke.width = 1.0;
            line(
                pm,
                CORNELL_CUE,
                24.0,
                CORNELL_CUE,
                page_h - CORNELL_SUMMARY,
                &paint,
                &stroke,
                t,
            );
            line(
                pm,
                24.0,
                page_h - CORNELL_SUMMARY,
                page_w - 24.0,
                page_h - CORNELL_SUMMARY,
                &paint,
                &stroke,
                t,
            );
            paint.set_color(rgb(look.paper_rule));
            stroke.width = 0.8;
            let mut y = 88.0;
            let last = page_h - CORNELL_SUMMARY - 8.0;
            while y < last {
                line(
                    pm,
                    CORNELL_CUE + 12.0,
                    y,
                    page_w - 24.0,
                    y,
                    &paint,
                    &stroke,
                    t,
                );
                y += 28.0;
            }
        }
        PaperKind::Grid => {
            paint.set_color(rgb(look.paper_rule));
            stroke.width = 0.6;
            let mut x = 24.0;
            while x < page_w {
                line(pm, x, 24.0, x, page_h - 24.0, &paint, &stroke, t);
                x += 24.0;
            }
            let mut y = 24.0;
            while y < page_h {
                line(pm, 24.0, y, page_w - 24.0, y, &paint, &stroke, t);
                y += 24.0;
            }
        }
        PaperKind::Dots => {
            paint.set_color(rgb(look.paper_rule_strong));
            let mut y = 32.0;
            while y < page_h - 16.0 {
                let mut x = 32.0;
                while x < page_w - 16.0 {
                    if let Some(path) = {
                        let mut pb = PathBuilder::new();
                        pb.push_circle(x, y, 0.9);
                        pb.finish()
                    } {
                        pm.fill_path(&path, &paint, FillRule::Winding, t, None);
                    }
                    x += 22.0;
                }
                y += 22.0;
            }
        }
        PaperKind::Millimetre => {
            stroke.width = 0.45;
            let mut y = 40.0;
            let mut i = 0i32;
            while y < page_h - 20.0 {
                paint.set_color(if i % 5 == 0 {
                    rgb(look.paper_rule_strong)
                } else {
                    rgb(look.paper_rule)
                });
                line(pm, 48.0, y, page_w - 20.0, y, &paint, &stroke, t);
                y += 8.0;
                i += 1;
            }
            let mut x = 48.0;
            let mut i = 0i32;
            while x < page_w - 20.0 {
                paint.set_color(if i % 5 == 0 {
                    rgb(look.paper_rule_strong)
                } else {
                    rgb(look.paper_rule)
                });
                line(pm, x, 40.0, x, page_h - 20.0, &paint, &stroke, t);
                x += 8.0;
                i += 1;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn line(
    pm: &mut Pixmap,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    paint: &Paint,
    stroke: &SkStroke,
    t: Transform,
) {
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y0);
    pb.line_to(x1, y1);
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, paint, stroke, t, None);
    }
}

fn rgb(c: egui::Color32) -> Color {
    Color::from_rgba8(c.r(), c.g(), c.b(), 255)
}

fn fill_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, color: egui::Color32, t: Transform) {
    let mut pb = PathBuilder::new();
    pb.push_rect(
        tiny_skia::Rect::from_xywh(x, y, w.max(1.0), h.max(1.0))
            .unwrap_or(tiny_skia::Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap()),
    );
    if let Some(path) = pb.finish() {
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(rgb(color));
        pm.fill_path(&path, &paint, FillRule::Winding, t, None);
    }
}

#[allow(clippy::too_many_arguments)]
fn blit(pm: &mut Pixmap, x: f32, y: f32, dw: f32, dh: f32, iw: u32, ih: u32, rgba: &[u8]) {
    let Some(src) = Pixmap::from_vec(
        rgba.to_vec(),
        tiny_skia::IntSize::from_wh(iw, ih).unwrap_or(tiny_skia::IntSize::from_wh(1, 1).unwrap()),
    ) else {
        return;
    };
    let sx = dw / iw as f32;
    let sy = dh / ih as f32;
    pm.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::from_translate(x, y).pre_scale(sx, sy),
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    pm: &mut Pixmap,
    font: &Font,
    text: &str,
    x: f32,
    y: f32,
    max_w: f32,
    size: f32,
    color: Color,
) {
    let mut cx = x;
    let mut cy = y + size;
    let line_h = size * 1.28;
    for ch in text.chars() {
        if ch == '\n' {
            cx = x;
            cy += line_h;
            continue;
        }
        let (metrics, bitmap) = font.rasterize(ch, size);
        if cx + metrics.advance_width > x + max_w && ch != ' ' {
            cx = x;
            cy += line_h;
        }
        stamp_glyph(
            pm,
            &bitmap,
            metrics.width,
            metrics.height,
            cx + metrics.xmin as f32,
            cy - metrics.height as f32 - metrics.ymin as f32,
            color,
        );
        cx += metrics.advance_width;
    }
}

fn stamp_glyph(pm: &mut Pixmap, bitmap: &[u8], gw: usize, gh: usize, x: f32, y: f32, color: Color) {
    let cr = color.red();
    let cg = color.green();
    let cb = color.blue();
    let width = pm.width();
    let height = pm.height();
    let data = pm.data_mut();
    for gy in 0..gh {
        for gx in 0..gw {
            let a = bitmap[gy * gw + gx] as f32 / 255.0;
            if a < 0.02 {
                continue;
            }
            let px = x as i32 + gx as i32;
            let py = y as i32 + gy as i32;
            if px < 0 || py < 0 || px as u32 >= width || py as u32 >= height {
                continue;
            }
            let i = ((py as u32 * width + px as u32) * 4) as usize;
            let inv = 1.0 - a;
            let dr = data[i] as f32 / 255.0;
            let dg = data[i + 1] as f32 / 255.0;
            let db = data[i + 2] as f32 / 255.0;
            let da = data[i + 3] as f32 / 255.0;
            data[i] = ((cr * a + dr * inv) * 255.0) as u8;
            data[i + 1] = ((cg * a + dg * inv) * 255.0) as u8;
            data[i + 2] = ((cb * a + db * inv) * 255.0) as u8;
            data[i + 3] = ((a + da * inv) * 255.0) as u8;
        }
    }
}

fn serif_font() -> Option<Font> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/liberation/LiberationSerif-Regular.ttf",
        "/usr/share/fonts/TTF/LiberationSerif-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
    ];
    for p in CANDIDATES {
        if let Ok(bytes) = std::fs::read(p) {
            if let Ok(f) = Font::from_bytes(bytes, FontSettings::default()) {
                return Some(f);
            }
        }
    }
    None
}

fn rgb_from_rgba(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    for px in data.chunks_exact(4) {
        out.extend_from_slice(&px[..3]);
    }
    out
}

fn simple_pdf(pages: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    let mut offsets = vec![0u32];
    let write_obj = |id: u32, payload: &[u8], body: &mut Vec<u8>, offsets: &mut Vec<u32>| {
        while offsets.len() <= id as usize {
            offsets.push(0);
        }
        offsets[id as usize] = body.len() as u32;
        body.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        body.extend_from_slice(payload);
        if !payload.ends_with(b"\n") {
            body.push(b'\n');
        }
        body.extend_from_slice(b"endobj\n");
    };

    let n = pages.len().max(1);
    let mut kids = String::from("[");
    // object layout: 1 catalog, 2 pages, 3..2+n page objs, then image+content pairs
    let pages_id = 2u32;
    let first_page_id = 3u32;
    let mut next = first_page_id + n as u32;
    let mut page_ids = Vec::new();
    let mut image_ids = Vec::new();
    let mut content_ids = Vec::new();
    for i in 0..n {
        page_ids.push(first_page_id + i as u32);
        kids.push_str(&format!("{} 0 R ", first_page_id + i as u32));
        image_ids.push(next);
        next += 1;
        content_ids.push(next);
        next += 1;
    }
    kids.push(']');

    body.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    write_obj(
        1,
        format!("<< /Type /Catalog /Pages {pages_id} 0 R >>\n").as_bytes(),
        &mut body,
        &mut offsets,
    );
    write_obj(
        pages_id,
        format!("<< /Type /Pages /Count {n} /Kids {kids} >>\n").as_bytes(),
        &mut body,
        &mut offsets,
    );

    for i in 0..n {
        let (w, h, rgb) = if pages.is_empty() {
            (595, 842, vec![255; 595 * 842 * 3])
        } else {
            let (w, h, ref rgb) = pages[i];
            (w, h, rgb.clone())
        };
        let pw = 595.0f32;
        let ph = pw * (h as f32 / w as f32);
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        let _ = enc.write_all(&rgb);
        let deflated = enc.finish().unwrap_or_default();
        let img_id = image_ids[i];
        let content_id = content_ids[i];
        let page_id = page_ids[i];
        write_obj(
            page_id,
            format!(
                "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im{i} {img_id} 0 R >> >> /Contents {content_id} 0 R >>\n"
            )
            .as_bytes(),
            &mut body,
            &mut offsets,
        );
        let header = format!(
            "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            deflated.len()
        );
        let mut payload = header.into_bytes();
        payload.extend_from_slice(&deflated);
        payload.extend_from_slice(b"\nendstream\n");
        write_obj(img_id, &payload, &mut body, &mut offsets);
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im{i} Do Q\n");
        let mut cpay = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        cpay.extend_from_slice(content.as_bytes());
        cpay.extend_from_slice(b"endstream\n");
        write_obj(content_id, &cpay, &mut body, &mut offsets);
    }

    let xref_at = body.len();
    let max_id = offsets.len() - 1;
    body.extend_from_slice(format!("xref\n0 {}\n", max_id + 1).as_bytes());
    body.extend_from_slice(b"0000000000 65535 f \n");
    for id in 1..=max_id {
        let off = offsets.get(id).copied().unwrap_or(0);
        body.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    body.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            max_id + 1
        )
        .as_bytes(),
    );
    body
}

fn stamp_original_pdf(
    note: &Note,
    look: &Look,
    media: &MediaLoader,
    note_dir: &std::path::Path,
) -> Option<Vec<u8>> {
    let source = note.pages.iter().find_map(|page| {
        page.background.as_ref().and_then(|bg| {
            bg.pdf_page?;
            Some(bg.file.clone())
        })
    })?;
    let bytes = std::fs::read(note_dir.join(&source)).ok()?;
    let mut doc = lopdf::Document::load_mem(&bytes).ok()?;
    let original: Vec<lopdf::ObjectId> = doc.page_iter().collect();
    let pages_id = doc
        .catalog()
        .ok()?
        .get(b"Pages")
        .and_then(lopdf::Object::as_reference)
        .ok()?;
    let mut out_ids = Vec::new();
    let mut used = std::collections::HashSet::new();
    for (i, page) in note.pages.iter().enumerate() {
        let pdf_slot =
            page.background.as_ref().and_then(
                |bg| {
                    if bg.file == source {
                        bg.pdf_page
                    } else {
                        None
                    }
                },
            );
        if let Some(index) = pdf_slot {
            if let Some(&id) = original.get(index as usize) {
                if used.insert(id) {
                    if page_has_marks(page) {
                        if let Some(layer) = annotation_layer(note, i, look, media, note_dir) {
                            let _ = stamp_page(&mut doc, id, &layer);
                        }
                    }
                    out_ids.push(id);
                    continue;
                }
            }
        }
        let pm = raster_page(note, i, look, 1.6, media, note_dir, RasterMode::Full)?;
        let (page_w, page_h) = note.page_size();
        let id = push_raster_page(
            &mut doc,
            pages_id,
            pm.width(),
            pm.height(),
            &straight_rgba(pm.data()),
            page_w * 72.0 / 96.0,
            page_h * 72.0 / 96.0,
        )?;
        out_ids.push(id);
    }
    if out_ids.is_empty() {
        return None;
    }
    set_page_list(&mut doc, pages_id, &out_ids).ok()?;
    let mut out = Vec::new();
    doc.save_to(&mut out).ok()?;
    Some(out)
}

fn page_has_marks(page: &crate::document::Page) -> bool {
    !page.strokes.is_empty() || !page.texts.is_empty() || !page.images.is_empty()
}

struct RgbaLayer {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn annotation_layer(
    note: &Note,
    page_i: usize,
    look: &Look,
    media: &MediaLoader,
    note_dir: &std::path::Path,
) -> Option<RgbaLayer> {
    let page = note.pages.get(page_i)?;
    let bg = page.background.as_ref()?;
    let pm = raster_page(
        note,
        page_i,
        look,
        2.0,
        media,
        note_dir,
        RasterMode::Annotations,
    )?;
    let (page_w, page_h) = note.page_size();
    let [x, y, w, h] = bg.fit(page_w, page_h);
    let scale = 2.0;
    let cropped = crop_rgba(
        pm.data(),
        pm.width(),
        pm.height(),
        (x * scale).round().max(0.0) as u32,
        (y * scale).round().max(0.0) as u32,
        (w * scale).round().max(1.0) as u32,
        (h * scale).round().max(1.0) as u32,
    )?;
    if !cropped.rgba.chunks_exact(4).any(|px| px[3] > 8) {
        return None;
    }
    Some(cropped)
}

fn crop_rgba(src: &[u8], sw: u32, sh: u32, x: u32, y: u32, w: u32, h: u32) -> Option<RgbaLayer> {
    if sw == 0 || sh == 0 {
        return None;
    }
    let x = x.min(sw.saturating_sub(1));
    let y = y.min(sh.saturating_sub(1));
    let w = w.min(sw - x).max(1);
    let h = h.min(sh - y).max(1);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * sw + x) * 4) as usize;
        let end = start + (w * 4) as usize;
        rgba.extend_from_slice(&src[start..end]);
    }
    Some(RgbaLayer {
        width: w,
        height: h,
        rgba: straight_rgba(&rgba),
    })
}

fn straight_rgba(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4) {
        let a = px[3];
        if a == 0 || a == 255 {
            out.extend_from_slice(px);
        } else {
            out.push(((u16::from(px[0]) * 255) / u16::from(a)) as u8);
            out.push(((u16::from(px[1]) * 255) / u16::from(a)) as u8);
            out.push(((u16::from(px[2]) * 255) / u16::from(a)) as u8);
            out.push(a);
        }
    }
    out
}

fn stamp_page(
    doc: &mut lopdf::Document,
    page_id: lopdf::ObjectId,
    layer: &RgbaLayer,
) -> lopdf::Result<()> {
    let (llx, lly, urx, ury, rotate) =
        page_box(doc, page_id).unwrap_or((0.0, 0.0, 595.0, 842.0, 0));
    let image_id = add_rgba_image(doc, layer)?;
    let name = insert_xobject(doc, page_id, image_id)?;
    let [a, b, c, d, e, f] = image_matrix(rotate, llx, lly, urx, ury);
    let ops = format!("q\n{a:.3} {b:.3} {c:.3} {d:.3} {e:.3} {f:.3} cm\n/{name} Do\nQ\n");
    doc.add_page_contents(page_id, ops.into_bytes())
}

fn image_matrix(rotate: i32, llx: f32, lly: f32, urx: f32, ury: f32) -> [f32; 6] {
    let w = urx - llx;
    let h = ury - lly;
    match rotate.rem_euclid(360) {
        90 => [0.0, h, -w, 0.0, llx + w, lly],
        180 => [-w, 0.0, 0.0, -h, llx + w, lly + h],
        270 => [0.0, -h, w, 0.0, llx, lly + h],
        _ => [w, 0.0, 0.0, h, llx, lly],
    }
}

fn page_box(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> Option<(f32, f32, f32, f32, i32)> {
    let mut current = page_id;
    let mut media = None;
    let mut crop = None;
    let mut rotate = 0i32;
    for _ in 0..16 {
        let dict = doc.get_dictionary(current).ok()?.clone();
        if media.is_none() {
            media = dict.get(b"MediaBox").ok().and_then(|obj| rect_of(doc, obj));
        }
        if crop.is_none() {
            crop = dict.get(b"CropBox").ok().and_then(|obj| rect_of(doc, obj));
        }
        if rotate == 0 {
            if let Ok(value) = dict.get(b"Rotate") {
                rotate = number(value).unwrap_or(0.0) as i32;
            }
        }
        let Ok(parent) = dict.get(b"Parent").and_then(lopdf::Object::as_reference) else {
            break;
        };
        current = parent;
    }
    let [llx, lly, urx, ury] = crop.or(media)?;
    Some((llx, lly, urx, ury, rotate))
}

fn rect_of(doc: &lopdf::Document, obj: &lopdf::Object) -> Option<[f32; 4]> {
    let obj = match obj {
        lopdf::Object::Reference(id) => doc.get_object(*id).ok()?,
        other => other,
    };
    let arr = obj.as_array().ok()?;
    if arr.len() != 4 {
        return None;
    }
    Some([
        number(&arr[0])?,
        number(&arr[1])?,
        number(&arr[2])?,
        number(&arr[3])?,
    ])
}

fn number(obj: &lopdf::Object) -> Option<f32> {
    match obj {
        lopdf::Object::Integer(n) => Some(*n as f32),
        lopdf::Object::Real(n) => Some(*n),
        _ => None,
    }
}

fn add_rgba_image(doc: &mut lopdf::Document, layer: &RgbaLayer) -> lopdf::Result<lopdf::ObjectId> {
    let mut rgb = Vec::with_capacity((layer.width * layer.height * 3) as usize);
    let mut alpha = Vec::with_capacity((layer.width * layer.height) as usize);
    for px in layer.rgba.chunks_exact(4) {
        rgb.extend_from_slice(&px[..3]);
        alpha.push(px[3]);
    }
    let mask = flate_stream(layer.width, layer.height, b"DeviceGray", &alpha, None);
    let mask_id = doc.add_object(lopdf::Object::Stream(mask));
    let image = flate_stream(layer.width, layer.height, b"DeviceRGB", &rgb, Some(mask_id));
    Ok(doc.add_object(lopdf::Object::Stream(image)))
}

fn flate_stream(
    width: u32,
    height: u32,
    color: &[u8],
    samples: &[u8],
    mask: Option<lopdf::ObjectId>,
) -> lopdf::Stream {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = enc.write_all(samples);
    let compressed = enc.finish().unwrap_or_default();
    let mut dict = lopdf::Dictionary::new();
    dict.set("Type", lopdf::Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", lopdf::Object::Name(b"Image".to_vec()));
    dict.set("Width", width as i64);
    dict.set("Height", height as i64);
    dict.set("ColorSpace", lopdf::Object::Name(color.to_vec()));
    dict.set("BitsPerComponent", 8_i64);
    dict.set("Filter", lopdf::Object::Name(b"FlateDecode".to_vec()));
    if let Some(mask) = mask {
        dict.set("SMask", lopdf::Object::Reference(mask));
    }
    let mut stream = lopdf::Stream::new(dict, compressed);
    stream.allows_compression = false;
    stream
}

fn insert_xobject(
    doc: &mut lopdf::Document,
    page_id: lopdf::ObjectId,
    image_id: lopdf::ObjectId,
) -> lopdf::Result<String> {
    let mut resources = nearest_resources(doc, page_id).unwrap_or_default();
    let mut xobjects = match resources.get(b"XObject") {
        Ok(lopdf::Object::Dictionary(dict)) => dict.clone(),
        Ok(lopdf::Object::Reference(id)) => doc
            .get_dictionary(*id)
            .ok()
            .cloned()
            .unwrap_or_else(lopdf::Dictionary::new),
        _ => lopdf::Dictionary::new(),
    };
    let mut name = "CahierInk".to_string();
    let mut n = 0u32;
    while xobjects.has(name.as_bytes()) {
        n += 1;
        name = format!("CahierInk{n}");
    }
    xobjects.set(name.as_str(), lopdf::Object::Reference(image_id));
    resources.set("XObject", xobjects);
    let page = doc.get_object_mut(page_id)?.as_dict_mut()?;
    page.set("Resources", resources);
    Ok(name)
}

fn nearest_resources(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> Option<lopdf::Dictionary> {
    let mut current = page_id;
    for _ in 0..16 {
        let dict = doc.get_dictionary(current).ok()?.clone();
        if let Ok(obj) = dict.get(b"Resources") {
            if let Some(resolved) = resolve_dict(doc, obj) {
                return Some(resolved);
            }
        }
        let Ok(parent) = dict.get(b"Parent").and_then(lopdf::Object::as_reference) else {
            break;
        };
        current = parent;
    }
    None
}

fn resolve_dict(doc: &lopdf::Document, obj: &lopdf::Object) -> Option<lopdf::Dictionary> {
    match obj {
        lopdf::Object::Dictionary(dict) => Some(dict.clone()),
        lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok().cloned(),
        _ => None,
    }
}

fn set_page_list(
    doc: &mut lopdf::Document,
    pages_id: lopdf::ObjectId,
    ids: &[lopdf::ObjectId],
) -> lopdf::Result<()> {
    for id in ids {
        if let Ok(page) = doc.get_object_mut(*id).and_then(lopdf::Object::as_dict_mut) {
            page.set("Parent", lopdf::Object::Reference(pages_id));
        }
    }
    let kids: Vec<lopdf::Object> = ids.iter().copied().map(lopdf::Object::Reference).collect();
    let pages = doc.get_object_mut(pages_id)?.as_dict_mut()?;
    pages.set("Kids", kids);
    pages.set("Count", ids.len() as i64);
    Ok(())
}

fn push_raster_page(
    doc: &mut lopdf::Document,
    pages_id: lopdf::ObjectId,
    width: u32,
    height: u32,
    rgba: &[u8],
    media_w: f32,
    media_h: f32,
) -> Option<lopdf::ObjectId> {
    let layer = RgbaLayer {
        width,
        height,
        rgba: rgba.to_vec(),
    };
    let image_id = add_rgba_image(doc, &layer).ok()?;
    let mut resources = lopdf::Dictionary::new();
    let mut xobjects = lopdf::Dictionary::new();
    xobjects.set("Im0", lopdf::Object::Reference(image_id));
    resources.set("XObject", xobjects);
    let content = format!("q\n{media_w:.2} 0 0 {media_h:.2} 0 0 cm\n/Im0 Do\nQ\n");
    let stream = lopdf::Stream::new(lopdf::Dictionary::new(), content.into_bytes());
    let content_id = doc.add_object(lopdf::Object::Stream(stream));
    let mut page = lopdf::Dictionary::new();
    page.set("Type", lopdf::Object::Name(b"Page".to_vec()));
    page.set("Parent", lopdf::Object::Reference(pages_id));
    page.set(
        "MediaBox",
        vec![
            lopdf::Object::Real(0.0),
            lopdf::Object::Real(0.0),
            lopdf::Object::Real(media_w),
            lopdf::Object::Real(media_h),
        ],
    );
    page.set("Resources", resources);
    page.set("Contents", lopdf::Object::Reference(content_id));
    Some(doc.add_object(lopdf::Object::Dictionary(page)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Note, PageBackground, PaperKind};
    use crate::ink::{InkPoint, InkStroke, Nib};
    use crate::look::Look;
    use crate::pdf::{marked_pdf, MarkedPage};
    use egui::{Color32, Pos2};

    fn mark(x: f32, y: f32) -> InkStroke {
        let mut stroke = InkStroke::new(Nib::Fineliner, Color32::from_rgb(220, 30, 30), 4.0);
        stroke.points.push(InkPoint::new(Pos2::new(x, y), 1.0));
        stroke
            .points
            .push(InkPoint::new(Pos2::new(x + 40.0, y + 10.0), 1.0));
        stroke
    }

    #[test]
    fn annotated_pdf_keeps_the_original_page_and_adds_ink() {
        let bytes = marked_pdf(&[MarkedPage {
            width: 200.0,
            height: 100.0,
            rotate: 0,
            marker: "KEEP-ORIGINAL",
        }]);
        let dir = std::env::temp_dir().join(format!("cahier-stamp-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("source.pdf"), &bytes).unwrap();
        let mut note = Note::blank("lecture", 0);
        note.paper = PaperKind::Blank;
        let (w, h) = crate::pdf::logical_size(200.0, 100.0);
        note.page_w = w;
        note.page_h = h;
        note.pages[0].background = Some(PageBackground {
            file: "source.pdf".into(),
            pdf_page: Some(0),
            source_size: Some([200.0, 100.0]),
        });
        note.pages[0].strokes.push(mark(30.0, 20.0));
        let look = Look::load();
        let media = MediaLoader {
            root: dir.join("media"),
        };
        let pdf = pages_pdf(&note, &look, &media, &dir).expect("export");
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains("KEEP-ORIGINAL"),
            "original page content was dropped"
        );
        assert!(
            text.contains("CahierInk"),
            "annotations were not added on top"
        );
        let doc = lopdf::Document::load_mem(&pdf).unwrap();
        assert_eq!(doc.get_pages().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotated_page_uses_a_counter_rotation() {
        let bytes = marked_pdf(&[MarkedPage {
            width: 200.0,
            height: 100.0,
            rotate: 90,
            marker: "ROT",
        }]);
        let [a, b, c, d, e, f] = image_matrix(90, 0.0, 0.0, 200.0, 100.0);
        assert_eq!([a, b, c, d, e, f], [0.0, 100.0, -200.0, 0.0, 200.0, 0.0]);
        let dir = std::env::temp_dir().join(format!("cahier-rot-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("source.pdf"), &bytes).unwrap();
        let mut note = Note::blank("rot", 0);
        note.paper = PaperKind::Blank;
        let (w, h) = crate::pdf::logical_size(100.0, 200.0);
        note.page_w = w;
        note.page_h = h;
        note.pages[0].background = Some(PageBackground {
            file: "source.pdf".into(),
            pdf_page: Some(0),
            source_size: Some([100.0, 200.0]),
        });
        note.pages[0].strokes.push(mark(12.0, 18.0));
        let pdf = pages_pdf(
            &note,
            &Look::load(),
            &MediaLoader {
                root: dir.join("media"),
            },
            &dir,
        )
        .unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("ROT"));
        assert!(text.contains("100.000 -200.000"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_note_still_exports() {
        let note = Note::blank("plain", 0);
        let dir = std::env::temp_dir();
        let pdf = pages_pdf(
            &note,
            &Look::load(),
            &MediaLoader {
                root: dir.to_path_buf(),
            },
            &dir,
        )
        .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }
}
