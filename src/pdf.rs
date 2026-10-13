//! PDF pages, rendered with [hayro](https://crates.io/crates/hayro).
//!
//! hayro is pure Rust and is linked into the binary. Cahier does not need
//! Poppler, MuPDF, or PDFium on the system.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf, PdfData};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, RenderSettings};

/// Screen points Cahier uses per PDF point (96 dpi paper, 72 dpi PDF).
pub const PAPER_PER_PT: f32 = 96.0 / 72.0;

#[derive(Clone, Copy, Debug)]
pub struct PdfPageSpec {
    pub width_pt: f32,
    pub height_pt: f32,
}

pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub scale: f32,
}

/// Logical page size for a PDF page, clamped so a poster still fits the lectern.
pub fn logical_size(width_pt: f32, height_pt: f32) -> (f32, f32) {
    let mut w = width_pt.max(1.0) * PAPER_PER_PT;
    let mut h = height_pt.max(1.0) * PAPER_PER_PT;
    let long = w.max(h);
    const MAX_SIDE: f32 = 2400.0;
    if long > MAX_SIDE {
        let s = MAX_SIDE / long;
        w *= s;
        h *= s;
    }
    (w.max(32.0), h.max(32.0))
}

/// Hayro scale (pixels per PDF point) for the current zoom, in a few steps.
#[cfg(test)]
pub fn render_scale(zoom: f32, pixels_per_point: f32) -> f32 {
    snap_scale(zoom.max(0.02) * pixels_per_point.max(0.5) * PAPER_PER_PT)
}

/// Scale from the on-screen rectangle. Tiny sheets still get a thumbnail.
pub fn raster_scale(
    screen_w: f32,
    screen_h: f32,
    src_w_pt: f32,
    src_h_pt: f32,
    pixels_per_point: f32,
) -> f32 {
    let ppp = pixels_per_point.max(0.5);
    let sx = (screen_w.max(1.0) * ppp) / src_w_pt.max(1.0);
    let sy = (screen_h.max(1.0) * ppp) / src_h_pt.max(1.0);
    snap_scale(sx.max(sy))
}

fn snap_scale(want: f32) -> f32 {
    let want = want.clamp(0.25, 8.0);
    const STEPS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0];
    STEPS
        .iter()
        .copied()
        .find(|step| *step + 0.08 >= want)
        .unwrap_or(8.0)
}

/// One PDF page to raster off the UI thread.
pub struct RasterJob {
    pub key: String,
    pub path: PathBuf,
    pub page: u32,
    pub scale: f32,
}

/// Background hayro worker. Keeps the last opened PDF so turning pages does not re-parse.
pub struct RasterHub {
    tx: Sender<RasterJob>,
    rx: Receiver<(String, Result<Rendered, String>)>,
}

impl RasterHub {
    pub fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("cahier-pdf".into())
            .spawn(move || raster_loop(job_rx, done_tx));
        Self {
            tx: job_tx,
            rx: done_rx,
        }
    }

    pub fn request(&self, job: RasterJob) -> bool {
        self.tx.send(job).is_ok()
    }

    pub fn take_ready(&self) -> Vec<(String, Result<Rendered, String>)> {
        let mut out = Vec::new();
        while let Ok(item) = self.rx.try_recv() {
            out.push(item);
        }
        out
    }
}

fn raster_loop(rx: Receiver<RasterJob>, tx: Sender<(String, Result<Rendered, String>)>) {
    let mut cached: Option<(PathBuf, Pdf)> = None;
    while let Ok(first) = rx.recv() {
        let mut latest: HashMap<(PathBuf, u32), RasterJob> = HashMap::new();
        latest.insert((first.path.clone(), first.page), first);
        while let Ok(job) = rx.try_recv() {
            latest.insert((job.path.clone(), job.page), job);
        }
        for job in latest.into_values() {
            if cached.as_ref().is_none_or(|(path, _)| path != &job.path) {
                cached = match std::fs::read(&job.path) {
                    Ok(bytes) => match open_pdf(bytes) {
                        Ok(pdf) => Some((job.path.clone(), pdf)),
                        Err(err) => {
                            if tx.send((job.key, Err(err))).is_err() {
                                return;
                            }
                            continue;
                        }
                    },
                    Err(err) => {
                        if tx.send((job.key, Err(err.to_string()))).is_err() {
                            return;
                        }
                        continue;
                    }
                };
            }
            let result = cached
                .as_ref()
                .map(|(_, pdf)| render_opened(pdf, job.page, job.scale))
                .unwrap_or_else(|| Err("This PDF could not be read.".into()));
            if tx.send((job.key, result)).is_err() {
                return;
            }
        }
    }
}

pub fn inspect_pdf(bytes: &[u8]) -> Result<Vec<PdfPageSpec>, String> {
    let pdf = open_pdf(bytes.to_vec())?;
    if pdf.pages().is_empty() {
        return Err("This PDF has no pages.".into());
    }
    Ok(pdf
        .pages()
        .iter()
        .map(|page| {
            let (w, h) = page.render_dimensions();
            PdfPageSpec {
                width_pt: w.max(1.0),
                height_pt: h.max(1.0),
            }
        })
        .collect())
}

pub fn render_pdf_file(path: &Path, page: u32, scale: f32) -> Result<Rendered, String> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    render_pdf_bytes(&bytes, page, scale)
}

pub fn render_pdf_bytes(bytes: &[u8], page: u32, scale: f32) -> Result<Rendered, String> {
    render_opened(&open_pdf(bytes.to_vec())?, page, scale)
}

fn render_opened(pdf: &Pdf, page: u32, scale: f32) -> Result<Rendered, String> {
    let pdf_page = pdf
        .pages()
        .get(page as usize)
        .ok_or_else(|| format!("PDF page {} is missing.", page + 1))?;
    let (pt_w, pt_h) = pdf_page.render_dimensions();
    let scale = clamp_scale(pt_w, pt_h, scale);
    let pixmap = render(
        pdf_page,
        &InterpreterSettings::default(),
        &RenderSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
            ..RenderSettings::default()
        },
    );
    let width = u32::from(pixmap.width());
    let height = u32::from(pixmap.height());
    if width == 0 || height == 0 {
        return Err("The PDF page rendered empty.".into());
    }
    Ok(Rendered {
        width,
        height,
        rgba: straight_rgba(pixmap.data_as_u8_slice()),
        scale,
    })
}

pub fn clamp_scale(pt_w: f32, pt_h: f32, scale: f32) -> f32 {
    let scale = scale.clamp(0.25, 8.0);
    let long = pt_w.max(pt_h).max(1.0) * scale;
    const MAX: f32 = 4096.0;
    if long > MAX {
        (scale * (MAX / long)).max(0.25)
    } else {
        scale
    }
}

/// Text baselines in PDF points, origin top-left of the page.
pub fn text_baselines(path: &Path, page: u32) -> Vec<f32> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(pdf) = open_pdf(bytes) else {
        return Vec::new();
    };
    let Some(pdf_page) = pdf.pages().get(page as usize) else {
        return Vec::new();
    };
    let (w, h) = pdf_page.render_dimensions();
    let settings = InterpreterSettings::default();
    let mut context = hayro::hayro_interpret::Context::new(
        kurbo::Affine::IDENTITY,
        kurbo::Rect::new(0.0, 0.0, w as f64, h as f64),
        pdf.xref(),
        settings,
    );
    let mut extractor = LineExtractor {
        height: h as f64,
        ys: Vec::new(),
    };
    hayro::hayro_interpret::interpret_page(pdf_page, &mut context, &mut extractor);
    cluster_baselines(&mut extractor.ys)
}

struct LineExtractor {
    height: f64,
    ys: Vec<f32>,
}

impl hayro::hayro_interpret::Device<'_> for LineExtractor {
    fn set_soft_mask(&mut self, _: Option<hayro::hayro_interpret::SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: hayro::hayro_interpret::BlendMode) {}
    fn draw_path(
        &mut self,
        _: &kurbo::BezPath,
        _: kurbo::Affine,
        _: &hayro::hayro_interpret::Paint<'_>,
        _: &hayro::hayro_interpret::PathDrawMode,
    ) {
    }
    fn push_clip_path(&mut self, _: &hayro::hayro_interpret::ClipPath) {}
    fn push_transparency_group(
        &mut self,
        _: f32,
        _: Option<hayro::hayro_interpret::SoftMask<'_>>,
        _: hayro::hayro_interpret::BlendMode,
    ) {
    }
    fn draw_glyph(
        &mut self,
        glyph: &hayro::hayro_interpret::font::Glyph<'_>,
        transform: kurbo::Affine,
        glyph_transform: kurbo::Affine,
        _: &hayro::hayro_interpret::Paint<'_>,
        _: &hayro::hayro_interpret::GlyphDrawMode,
    ) {
        if glyph.as_unicode().is_none() {
            return;
        }
        let flip = kurbo::Affine::translate((0.0, self.height))
            * kurbo::Affine::scale_non_uniform(1.0, -1.0);
        let p = flip * transform * glyph_transform * kurbo::Point::new(0.0, 0.0);
        self.ys.push(p.y as f32);
    }
    fn draw_image(&mut self, _: hayro::hayro_interpret::Image<'_, '_>, _: kurbo::Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn cluster_baselines(ys: &mut [f32]) -> Vec<f32> {
    if ys.is_empty() {
        return Vec::new();
    }
    ys.sort_by(|a, b| a.total_cmp(b));
    let mut out = Vec::new();
    let mut acc = ys[0];
    let mut n = 1u32;
    for &y in &ys[1..] {
        if (y - acc / n as f32).abs() < 2.4 {
            acc += y;
            n += 1;
        } else {
            out.push(acc / n as f32);
            acc = y;
            n = 1;
        }
    }
    out.push(acc / n as f32);
    out
}

fn open_pdf(bytes: Vec<u8>) -> Result<Pdf, String> {
    let data: PdfData = Arc::new(bytes);
    Pdf::new(data).map_err(|err| match err {
        LoadPdfError::Decryption(_) => "This PDF is encrypted.".into(),
        LoadPdfError::Invalid => "This PDF could not be read.".into(),
    })
}

fn straight_rgba(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4) {
        let a = px[3];
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
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

#[cfg(test)]
pub(crate) struct MarkedPage {
    pub width: f32,
    pub height: f32,
    pub rotate: i32,
    pub marker: &'static str,
}

/// A tiny valid PDF whose content streams contain the given markers.
#[cfg(test)]
pub(crate) fn marked_pdf(pages: &[MarkedPage]) -> Vec<u8> {
    let n = pages.len().max(1);
    let mut body = Vec::new();
    body.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = vec![0u32; 3 + n * 2];
    let mut kids = String::new();
    for i in 0..n {
        let page_id = 3 + i * 2;
        if i > 0 {
            kids.push(' ');
        }
        kids.push_str(&format!("{page_id} 0 R"));
    }
    push_obj(
        &mut body,
        &mut offsets,
        1,
        "<< /Type /Catalog /Pages 2 0 R >>",
    );
    push_obj(
        &mut body,
        &mut offsets,
        2,
        &format!("<< /Type /Pages /Count {n} /Kids [{kids}] >>"),
    );
    for (i, page) in pages.iter().enumerate() {
        let page_id = 3 + i * 2;
        let content_id = page_id + 1;
        let stream = format!("% {}\n", page.marker);
        let rotate = if page.rotate == 0 {
            String::new()
        } else {
            format!(" /Rotate {}", page.rotate)
        };
        push_obj(
            &mut body,
            &mut offsets,
            page_id as u32,
            &format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}]{rotate} /Contents {content_id} 0 R /Resources << >> >>",
                w = page.width,
                h = page.height,
            ),
        );
        push_obj(
            &mut body,
            &mut offsets,
            content_id as u32,
            &format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
        );
    }
    let xref_at = body.len();
    let size = offsets.len();
    body.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    body.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        body.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    body.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n")
            .as_bytes(),
    );
    body
}

#[cfg(test)]
fn push_obj(body: &mut Vec<u8>, offsets: &mut [u32], id: u32, payload: &str) {
    offsets[id as usize] = body.len() as u32;
    body.extend_from_slice(format!("{id} 0 obj\n{payload}\nendobj\n").as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_and_render_a_marked_page() {
        let bytes = marked_pdf(&[MarkedPage {
            width: 200.0,
            height: 100.0,
            rotate: 0,
            marker: "HELLO-PAGE",
        }]);
        let specs = inspect_pdf(&bytes).expect("inspect");
        assert_eq!(specs.len(), 1);
        assert!((specs[0].width_pt - 200.0).abs() < 0.5);
        assert!((specs[0].height_pt - 100.0).abs() < 0.5);
        let rendered = render_pdf_bytes(&bytes, 0, 1.0).expect("render");
        assert!(rendered.width >= 180 && rendered.width <= 220);
        assert!(rendered.height >= 80 && rendered.height <= 120);
        assert_eq!(
            rendered.rgba.len(),
            rendered.width as usize * rendered.height as usize * 4
        );
        let (w, h) = logical_size(specs[0].width_pt, specs[0].height_pt);
        assert!((w / h - 2.0).abs() < 0.02);
    }

    #[test]
    fn two_pages_keep_their_index() {
        let bytes = marked_pdf(&[
            MarkedPage {
                width: 100.0,
                height: 200.0,
                rotate: 0,
                marker: "ONE",
            },
            MarkedPage {
                width: 300.0,
                height: 100.0,
                rotate: 0,
                marker: "TWO",
            },
        ]);
        let specs = inspect_pdf(&bytes).unwrap();
        assert_eq!(specs.len(), 2);
        assert!(specs[0].height_pt > specs[0].width_pt);
        assert!(specs[1].width_pt > specs[1].height_pt);
    }

    #[test]
    fn render_scale_steps_up_with_zoom() {
        assert_eq!(render_scale(0.5, 1.0), 0.75);
        assert_eq!(render_scale(1.0, 1.0), 1.5);
        assert!(render_scale(3.0, 1.0) >= 4.0);
        assert_eq!(render_scale(0.05, 1.0), 0.25);
    }

    #[test]
    fn tiny_on_screen_pages_still_get_a_thumbnail() {
        assert_eq!(raster_scale(20.0, 20.0, 600.0, 800.0, 1.0), 0.25);
        let scale = raster_scale(800.0, 600.0, 600.0, 450.0, 1.0);
        assert!((1.0..=2.0).contains(&scale));
    }

    #[test]
    fn raster_hub_keeps_page_index_off_the_ui_thread() {
        let dir = std::env::temp_dir().join(format!(
            "cahier-pdf-hub-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("talk.pdf");
        let bytes = marked_pdf(&[
            MarkedPage {
                width: 120.0,
                height: 80.0,
                rotate: 0,
                marker: "A",
            },
            MarkedPage {
                width: 80.0,
                height: 120.0,
                rotate: 0,
                marker: "B",
            },
        ]);
        std::fs::write(&path, bytes).unwrap();
        let hub = RasterHub::new();
        assert!(hub.request(RasterJob {
            key: "p1".into(),
            path: path.clone(),
            page: 1,
            scale: 1.0,
        }));
        let mut got = None;
        for _ in 0..200 {
            if let Some(item) = hub.take_ready().into_iter().next() {
                got = Some(item);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (key, rendered) = got.expect("worker result");
        assert_eq!(key, "p1");
        let page = rendered.expect("page 2");
        assert!(page.height >= page.width);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
