//! PDF pages for the lectern.
//!
//! Progressive, zoom-aware PDF backgrounds. Poppler sends uncompressed pixels
//! through a pipe; hayro is the built-in fallback. Decoding, rasterization and
//! texture preparation stay on a worker, with a viewport-pruned work queue.

#[cfg(test)]
#[path = "pdf/bench.rs"]
mod bench;

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf, PdfData};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, RenderSettings};

/// Screen points Inkarchy uses per PDF point (96 dpi paper, 72 dpi PDF).
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

/// Prepared on the worker, so uploading a result does not convert millions of
/// pixels in the drawing/event loop.
pub struct RasterImage {
    pub image: egui::ColorImage,
    pub scale: f32,
}

impl From<&Rendered> for RasterImage {
    fn from(rendered: &Rendered) -> Self {
        Self {
            image: egui::ColorImage::from_rgba_unmultiplied(
                [rendered.width as usize, rendered.height as usize],
                &rendered.rgba,
            ),
            scale: rendered.scale,
        }
    }
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

/// Up to 432 dpi, subject to the per-page pixel budget below.
pub const DISPLAY_SCALE_MAX: f32 = 6.0;
const MAX_RASTER_SIDE: f32 = 4096.0;
const MAX_RASTER_PIXELS: f32 = 8.0 * 1024.0 * 1024.0;

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
    display_scale(sx.max(sy))
}

/// Discrete resolution levels avoid re-rendering for every pinch increment.
pub fn display_scale(wanted: f32) -> f32 {
    snap_scale(wanted.min(DISPLAY_SCALE_MAX))
}

/// Readable first paint before the full-resolution pass, for either engine.
pub fn preview_scale(wanted: f32) -> f32 {
    wanted.min(0.5)
}

fn snap_scale(want: f32) -> f32 {
    let want = want.clamp(0.25, DISPLAY_SCALE_MAX);
    const STEPS: [f32; 10] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 6.0];
    STEPS
        .iter()
        .copied()
        .find(|step| *step >= want)
        .unwrap_or(DISPLAY_SCALE_MAX)
}

/// One PDF page to raster off the UI thread.
pub struct RasterJob {
    pub key: String,
    pub path: PathBuf,
    pub page: u32,
    pub scale: f32,
    /// Try the full-size disk cache first; otherwise return a cheap first paint.
    pub preview: bool,
}

#[derive(Default)]
struct RasterQueue {
    jobs: VecDeque<RasterJob>,
    repaint: Option<egui::Context>,
    stopped: bool,
}

impl RasterQueue {
    fn pop(&mut self) -> Option<RasterJob> {
        // Blank visible pages get their preview before any detail upgrades.
        let i = self.jobs.iter().position(|job| job.preview).unwrap_or(0);
        self.jobs.remove(i)
    }

    fn retain(&mut self, visible: &HashSet<String>) -> Vec<String> {
        let mut removed = Vec::new();
        self.jobs.retain(|job| {
            if visible.contains(&job.key) {
                true
            } else {
                removed.push(job.key.clone());
                false
            }
        });
        removed
    }
}

type SharedRasterQueue = Arc<(Mutex<RasterQueue>, Condvar)>;
type RasterResult = (String, Result<RasterImage, String>);

/// One bounded worker: a large document cannot queue unlimited pixel buffers.
pub struct RasterHub {
    queue: SharedRasterQueue,
    rx: Receiver<RasterResult>,
}

impl RasterHub {
    pub fn new() -> Self {
        let queue = Arc::new((Mutex::new(RasterQueue::default()), Condvar::new()));
        let worker_queue = queue.clone();
        let (done_tx, done_rx) = mpsc::sync_channel(2);
        if thread::Builder::new()
            .name("inkarchy-pdf".into())
            .spawn(move || raster_loop(worker_queue, done_tx))
            .is_err()
        {
            queue.0.lock().unwrap().stopped = true;
        }
        Self { queue, rx: done_rx }
    }

    pub fn request(&self, job: RasterJob) -> bool {
        let mut queue = self.queue.0.lock().unwrap();
        if queue.stopped {
            return false;
        }
        if let Some(i) = queue.jobs.iter().position(|old| old.key == job.key) {
            queue.jobs.remove(i);
        }
        if queue.jobs.len() >= 64 {
            return false;
        }
        queue.jobs.push_back(job);
        self.queue.1.notify_one();
        true
    }

    pub fn set_repaint_context(&self, ctx: &egui::Context) {
        self.queue.0.lock().unwrap().repaint = Some(ctx.clone());
    }

    pub fn retain_visible(&self, visible: &HashSet<String>) -> Vec<String> {
        self.queue.0.lock().unwrap().retain(visible)
    }

    pub fn take_ready(&self) -> Vec<RasterResult> {
        // At most one texture upload per frame. The worker wakes the next frame.
        self.rx.try_recv().into_iter().collect()
    }
}

impl Drop for RasterHub {
    fn drop(&mut self) {
        let mut queue = self.queue.0.lock().unwrap();
        queue.stopped = true;
        queue.jobs.clear();
        self.queue.1.notify_one();
    }
}

fn raster_loop(queue: SharedRasterQueue, tx: SyncSender<RasterResult>) {
    let mut cached: Option<(PathBuf, Pdf)> = None;
    loop {
        let job = {
            let mut state = queue.0.lock().unwrap();
            while state.jobs.is_empty() && !state.stopped {
                state = queue.1.wait(state).unwrap();
            }
            if state.stopped {
                return;
            }
            state.pop().unwrap()
        };
        let result = load_raster_cache(&job.path, job.page, job.scale)
            .map(Ok)
            .unwrap_or_else(|| {
                let scale = if job.preview {
                    preview_scale(job.scale)
                } else {
                    job.scale
                };
                if let Some(hit) = load_raster_cache(&job.path, job.page, scale) {
                    return Ok(hit);
                }
                render_screen_page(&job.path, job.page, scale, &mut cached)
            });
        let image = result.as_ref().map(RasterImage::from).map_err(Clone::clone);
        if tx.send((job.key, image)).is_err() {
            return;
        }
        let ctx = queue.0.lock().unwrap().repaint.clone();
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
        // Publish pixels before compression/fsync, so disk latency does not
        // delay first paint. Existing cache hits do not need to be rewritten.
        if let Ok(rendered) = &result {
            if !raster_cache_fresh(&job.path, job.page, rendered.scale) {
                save_raster_cache(&job.path, job.page, rendered);
            }
        }
    }
}

fn render_screen_page(
    path: &Path,
    page: u32,
    scale: f32,
    cached: &mut Option<(PathBuf, Pdf)>,
) -> Result<Rendered, String> {
    if cached.as_ref().is_none_or(|(p, _)| p != path) {
        *cached = match std::fs::read(path) {
            Ok(bytes) => {
                let pdf = open_pdf(bytes)?;
                Some((path.to_path_buf(), pdf))
            }
            Err(err) => return Err(err.to_string()),
        };
    }
    cached
        .as_ref()
        .map(|(_, pdf)| {
            let pdf_page = pdf
                .pages()
                .get(page as usize)
                .ok_or_else(|| format!("PDF page {} is missing.", page + 1))?;
            let (w, h) = pdf_page.render_dimensions();
            let scale = clamp_scale(w, h, scale);
            render_poppler(path, page, scale)
                .map(Ok)
                .unwrap_or_else(|| render_opened(pdf, page, scale))
        })
        .unwrap_or_else(|| Err("This PDF could not be read.".into()))
}

pub fn poppler_available() -> bool {
    static YES: OnceLock<bool> = OnceLock::new();
    *YES.get_or_init(|| {
        Command::new("pdftoppm")
            .arg("-v")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    })
}

fn render_poppler(path: &Path, page: u32, scale: f32) -> Option<Rendered> {
    if !poppler_available() {
        return None;
    }
    // Omitting the output prefix streams PPM to stdout: no temporary file,
    // PNG compression, directory scan, or subsequent PNG decompression.
    let dpi = (scale * 72.0).to_string();
    let page_1 = page.checked_add(1)?.to_string();
    let output = Command::new("pdftoppm")
        .args([
            "-singlefile",
            "-cropbox",
            "-f",
            &page_1,
            "-l",
            &page_1,
            "-r",
            &dpi,
            "-aa",
            "yes",
            "-aaVector",
            "yes",
        ])
        .arg(path)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let img = image::load_from_memory_with_format(&output.stdout, image::ImageFormat::Pnm)
        .ok()?
        .into_rgba8();
    let width = img.width();
    let height = img.height();
    if width == 0 || height == 0 {
        return None;
    }
    Some(Rendered {
        width,
        height,
        rgba: img.into_raw(),
        scale,
    })
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
    let w = pt_w.max(1.0);
    let h = pt_h.max(1.0);
    // Never restore a minimum scale after bounding a poster: that used to
    // defeat the allocation limit for very large PDF page dimensions.
    // Leave rounding room: engines ceil dimensions after converting to dpi.
    scale
        .clamp(0.001, DISPLAY_SCALE_MAX)
        .min((MAX_RASTER_SIDE - 1.0) / w.max(h))
        .min(((MAX_RASTER_PIXELS - 2.0 * MAX_RASTER_SIDE) / (w * h)).sqrt())
}

fn raster_cache_path(pdf: &Path, page: u32, scale: f32) -> PathBuf {
    // Scope to the source file, including size and mtime. Different PDFs in
    // one directory must never share pixels, nor reuse pre-cropbox rasters.
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    pdf.file_name().hash(&mut hash);
    if let Ok(meta) = std::fs::metadata(pdf) {
        meta.len().hash(&mut hash);
        meta.modified().ok().hash(&mut hash);
    }
    pdf.parent().unwrap_or(pdf).join("raster").join(format!(
        "v2-{:016x}-p{page}-{:08x}.png",
        hash.finish(),
        scale.to_bits()
    ))
}

pub fn raster_cache_fresh(pdf: &Path, page: u32, scale: f32) -> bool {
    let cache = raster_cache_path(pdf, page, scale);
    let Ok(pdf_meta) = std::fs::metadata(pdf) else {
        return false;
    };
    let Ok(cache_meta) = std::fs::metadata(&cache) else {
        return false;
    };
    match (cache_meta.modified(), pdf_meta.modified()) {
        (Ok(cached), Ok(src)) => cached >= src,
        _ => true,
    }
}

pub fn load_raster_cache(pdf: &Path, page: u32, scale: f32) -> Option<Rendered> {
    if !raster_cache_fresh(pdf, page, scale) {
        return None;
    }
    let img = image::open(raster_cache_path(pdf, page, scale))
        .ok()?
        .into_rgba8();
    let width = img.width();
    let height = img.height();
    if width == 0 || height == 0 {
        return None;
    }
    Some(Rendered {
        width,
        height,
        rgba: img.into_raw(),
        scale,
    })
}

fn save_raster_cache(pdf: &Path, page: u32, rendered: &Rendered) {
    // High zoom levels live in the GPU LRU, not in a growing stack of disk
    // images. Keep previews and normal reading resolutions for fast reopen.
    if rendered.scale > 2.0 {
        return;
    }
    let path = raster_cache_path(pdf, page, rendered.scale);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::ImageEncoder;
    let mut encoded = Vec::new();
    if PngEncoder::new_with_quality(&mut encoded, CompressionType::Fast, FilterType::Sub)
        .write_image(
            &rendered.rgba,
            rendered.width,
            rendered.height,
            image::ExtendedColorType::Rgba8,
        )
        .is_ok()
    {
        let _ = crate::library::atomic_write(&path, &encoded);
    }
}

/// Keep the current raster until the display target is clearly sharper.
pub fn raster_is_sharp_enough(have: f32, wanted: f32) -> bool {
    if have <= 0.0 {
        return false;
    }
    let target = wanted.min(DISPLAY_SCALE_MAX);
    target <= 0.0 || target <= have * 1.05
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

/// Unicode from embedded PDF fonts (ToUnicode / encoding). Empty on scans.
pub fn extract_text(path: &Path, page: u32) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    extract_text_from_bytes(&bytes, page)
}

fn extract_text_from_bytes(bytes: &[u8], page: u32) -> String {
    let Ok(pdf) = open_pdf(bytes.to_vec()) else {
        return String::new();
    };
    extract_text_from_pdf(&pdf, page)
}

fn extract_text_from_pdf(pdf: &Pdf, page: u32) -> String {
    let Some(pdf_page) = pdf.pages().get(page as usize) else {
        return String::new();
    };
    let (w, h) = pdf_page.render_dimensions();
    let settings = InterpreterSettings::default();
    let mut context = hayro::hayro_interpret::Context::new(
        kurbo::Affine::IDENTITY,
        kurbo::Rect::new(0.0, 0.0, w as f64, h as f64),
        pdf.xref(),
        settings,
    );
    let mut extractor = GlyphExtractor {
        height: h as f64,
        glyphs: Vec::new(),
    };
    hayro::hayro_interpret::interpret_page(pdf_page, &mut context, &mut extractor);
    glyphs_to_string(extractor.glyphs)
}

struct GlyphExtractor {
    height: f64,
    glyphs: Vec<(f32, f32, char)>,
}

impl hayro::hayro_interpret::Device<'_> for GlyphExtractor {
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
        let Some(ch) = glyph.as_unicode() else {
            return;
        };
        let flip = kurbo::Affine::translate((0.0, self.height))
            * kurbo::Affine::scale_non_uniform(1.0, -1.0);
        let p = flip * transform * glyph_transform * kurbo::Point::new(0.0, 0.0);
        self.glyphs.push((p.x as f32, p.y as f32, ch));
    }
    fn draw_image(&mut self, _: hayro::hayro_interpret::Image<'_, '_>, _: kurbo::Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn glyphs_to_string(mut glyphs: Vec<(f32, f32, char)>) -> String {
    if glyphs.is_empty() {
        return String::new();
    }
    glyphs.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
    let mut out = String::new();
    let mut line_y = glyphs[0].1;
    let mut last_x = f32::NEG_INFINITY;
    for (x, y, ch) in glyphs {
        if (y - line_y).abs() > 3.0 {
            out.push('\n');
            line_y = y;
        } else if x - last_x > 10.0 && !out.is_empty() && !out.ends_with('\n') {
            out.push(' ');
        }
        out.push(ch);
        last_x = x;
    }
    out
}

/// Background hayro text pass. Keeps the last opened PDF so shelf search
/// does not re-parse on every page.
pub struct TextJob {
    pub key: String,
    pub path: PathBuf,
    pub page: u32,
}

pub struct TextHub {
    tx: Sender<TextJob>,
    rx: Receiver<(String, String)>,
}

impl TextHub {
    pub fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("inkarchy-pdf-text".into())
            .spawn(move || text_loop(job_rx, done_tx));
        Self {
            tx: job_tx,
            rx: done_rx,
        }
    }

    pub fn request(&self, job: TextJob) -> bool {
        self.tx.send(job).is_ok()
    }

    pub fn take_ready(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        while let Ok(item) = self.rx.try_recv() {
            out.push(item);
        }
        out
    }
}

fn text_loop(rx: Receiver<TextJob>, tx: Sender<(String, String)>) {
    let mut cached: Option<(PathBuf, Pdf)> = None;
    loop {
        let Ok(job) = rx.recv() else {
            return;
        };
        if cached
            .as_ref()
            .map(|(path, _)| path != &job.path)
            .unwrap_or(true)
        {
            cached = std::fs::read(&job.path)
                .ok()
                .and_then(|bytes| open_pdf(bytes).ok())
                .map(|pdf| (job.path.clone(), pdf));
        }
        let text = cached
            .as_ref()
            .filter(|(path, _)| path == &job.path)
            .map(|(_, pdf)| extract_text_from_pdf(pdf, job.page))
            .unwrap_or_else(|| extract_text(&job.path, job.page));
        if tx.send((job.key, text)).is_err() {
            return;
        }
    }
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
    if src.chunks_exact(4).all(|px| px[3] == 255) {
        return src.to_vec();
    }
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

/// Helvetica Type1 page so hayro can map glyphs to Unicode.
#[cfg(test)]
pub(crate) fn text_pdf(text: &str) -> Vec<u8> {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)");
    let stream = format!("BT /F1 12 Tf 20 40 Td ({escaped}) Tj ET\n");
    let mut body = Vec::new();
    body.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = vec![0u32; 6];
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
        "<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
    );
    push_obj(
        &mut body,
        &mut offsets,
        3,
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
    );
    push_obj(
        &mut body,
        &mut offsets,
        4,
        &format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
    );
    push_obj(
        &mut body,
        &mut offsets,
        5,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    );
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

    struct TempPdf(PathBuf);

    impl TempPdf {
        fn new(bytes: &[u8]) -> Self {
            let dir = std::env::temp_dir().join(format!("inkarchy-pdf-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("source.pdf");
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
    }

    impl Drop for TempPdf {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    fn receive(hub: &RasterHub) -> (String, RasterImage) {
        let (key, result) = hub
            .rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("PDF worker");
        (key, result.expect("rendered page"))
    }

    #[test]
    fn worker_returns_preview_then_real_zoom_detail_and_uses_disk_cache() {
        let file = TempPdf::new(&text_pdf("Small text must stay sharp"));
        let hub = RasterHub::new();
        assert!(hub.request(RasterJob {
            key: "visible".into(),
            path: file.0.clone(),
            page: 0,
            scale: 3.0,
            preview: true,
        }));
        let (_, preview) = receive(&hub);
        assert_eq!(preview.scale, 0.5);
        assert!(hub.request(RasterJob {
            key: "visible".into(),
            path: file.0.clone(),
            page: 0,
            scale: 3.0,
            preview: false,
        }));
        let (_, detail) = receive(&hub);
        assert_eq!(detail.scale, 3.0);
        assert_eq!(detail.image.width(), preview.image.width() * 6);
        assert!(detail.image.pixels.iter().any(|px| px.r() < 100));

        // A cached full-quality page bypasses the preview, even when the job
        // is the first request for a page without a resident texture.
        let cached = render_pdf_file(&file.0, 0, 1.5).unwrap();
        save_raster_cache(&file.0, 0, &cached);
        assert!(hub.request(RasterJob {
            key: "reopened".into(),
            path: file.0.clone(),
            page: 0,
            scale: 1.5,
            preview: true,
        }));
        let (_, reopened) = receive(&hub);
        assert_eq!(reopened.scale, 1.5);
        assert_eq!(
            reopened.image.pixels,
            RasterImage::from(&cached).image.pixels
        );
    }

    #[test]
    fn viewport_discards_old_pages_and_previews_precede_detail() {
        let job = |key: &str, page, preview| RasterJob {
            key: key.into(),
            path: "source.pdf".into(),
            page,
            scale: 2.0,
            preview,
        };
        let mut queue = RasterQueue::default();
        queue.jobs.extend([
            job("old-page", 0, true),
            job("detail", 1, false),
            job("new-page", 90, true),
        ]);
        let visible = ["detail".to_string(), "new-page".to_string()]
            .into_iter()
            .collect();
        assert_eq!(queue.retain(&visible), vec!["old-page"]);
        assert_eq!(queue.pop().unwrap().key, "new-page");
        assert_eq!(queue.pop().unwrap().key, "detail");
        assert!(queue.pop().is_none());
    }

    #[test]
    fn cache_is_scoped_to_source_and_invalidated_when_source_changes() {
        let file = TempPdf::new(&text_pdf("First"));
        let rendered = render_pdf_file(&file.0, 0, 1.0).unwrap();
        save_raster_cache(&file.0, 0, &rendered);
        assert!(load_raster_cache(&file.0, 0, 1.0).is_some());
        let other = file.0.with_file_name("other.pdf");
        std::fs::write(&other, text_pdf("Second")).unwrap();
        assert!(load_raster_cache(&other, 0, 1.0).is_none());
        std::fs::write(
            &file.0,
            text_pdf("Changed document with a different length"),
        )
        .unwrap();
        assert!(load_raster_cache(&file.0, 0, 1.0).is_none());
    }

    #[test]
    fn poppler_and_hayro_agree_on_rotated_crop_and_fractional_dpi() {
        if !poppler_available() {
            return;
        }
        let mut doc = lopdf::Document::load_mem(&text_pdf("Crop")).unwrap();
        let page_id = doc.get_pages()[&1];
        let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
        page.set("CropBox", vec![10.into(), 10.into(), 170.into(), 90.into()]);
        page.set("Rotate", 90);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let file = TempPdf::new(&bytes);
        for scale in [0.5, 1.337, 3.0] {
            let poppler = render_poppler(&file.0, 0, scale).unwrap();
            let hayro = render_pdf_bytes(&bytes, 0, scale).unwrap();
            assert!(poppler.width.abs_diff(hayro.width) <= 1);
            assert!(poppler.height.abs_diff(hayro.height) <= 1);
            assert!(poppler.height > poppler.width);
            assert!(poppler.rgba.chunks_exact(4).any(|px| px[0] < 100));
        }
    }

    #[test]
    fn extract_text_reads_helvetica_glyphs() {
        let bytes = text_pdf("Fourier");
        let text = extract_text_from_bytes(&bytes, 0);
        let folded: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            folded.to_lowercase().contains("fourier"),
            "extracted {text:?}"
        );
        let dir = std::env::temp_dir().join(format!(
            "inkarchy-pdf-text-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("page.pdf");
        std::fs::write(&path, &bytes).unwrap();
        let from_path = extract_text(&path, 0);
        let _ = std::fs::remove_dir_all(&dir);
        let folded_path: String = from_path.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(folded_path.to_lowercase().contains("fourier"));
    }

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
        assert_eq!(render_scale(3.0, 1.0), 4.0);
        assert_eq!(render_scale(0.05, 1.0), 0.25);
    }

    #[test]
    fn tiny_on_screen_pages_still_get_a_thumbnail() {
        assert_eq!(raster_scale(20.0, 20.0, 600.0, 800.0, 1.0), 0.25);
        let scale = raster_scale(800.0, 600.0, 600.0, 450.0, 1.0);
        assert!((1.0..=DISPLAY_SCALE_MAX).contains(&scale));
    }

    #[test]
    fn zoom_and_hidpi_get_real_detail_with_bounded_allocations() {
        assert_eq!(preview_scale(4.0), 0.5);
        assert_eq!(display_scale(4.0), 4.0);
        assert_eq!(raster_scale(800.0, 1100.0, 595.0, 842.0, 2.0), 3.0);
        assert_eq!(
            raster_scale(4000.0, 4000.0, 100.0, 100.0, 2.0),
            DISPLAY_SCALE_MAX
        );
        let a4 = clamp_scale(595.0, 842.0, 8.0);
        assert!(a4 > 4.0 && a4 < 4.1, "a4 clamped to {a4}");
        assert!((clamp_scale(595.0, 842.0, 1.0) - 1.0).abs() < 0.001);
        assert!(!raster_is_sharp_enough(1.5, 4.0));
        for (w, h) in [(595.0, 842.0), (14400.0, 14400.0), (100000.0, 1000.0)] {
            let scale = clamp_scale(w, h, 6.0);
            assert!((w.max(h) * scale).ceil() <= MAX_RASTER_SIDE);
            assert!((w * scale).ceil() * (h * scale).ceil() <= MAX_RASTER_PIXELS);
        }
        assert!(raster_is_sharp_enough(1.5, 1.5));
        assert!(!raster_is_sharp_enough(0.25, 1.5));
        assert!(!raster_is_sharp_enough(1.0, 1.5));
    }

    #[test]
    fn poppler_renders_a_marked_page_when_available() {
        if !poppler_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "inkarchy-poppler-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("talk.pdf");
        std::fs::write(
            &path,
            marked_pdf(&[MarkedPage {
                width: 200.0,
                height: 100.0,
                rotate: 0,
                marker: "POP",
            }]),
        )
        .unwrap();
        let rendered = render_poppler(&path, 0, 1.0).expect("pdftoppm");
        assert!(rendered.width >= 180);
        assert!(rendered.height >= 80);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn raster_cache_roundtrip_skips_hayro() {
        let dir = std::env::temp_dir().join(format!(
            "inkarchy-pdf-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("talk.pdf");
        let bytes = marked_pdf(&[MarkedPage {
            width: 80.0,
            height: 120.0,
            rotate: 0,
            marker: "C",
        }]);
        std::fs::write(&path, bytes).unwrap();
        let rendered = render_pdf_file(&path, 0, 0.25).unwrap();
        save_raster_cache(&path, 0, &rendered);
        assert!(raster_cache_fresh(&path, 0, 0.25));
        let hit = load_raster_cache(&path, 0, 0.25).expect("cache");
        assert_eq!(hit.width, rendered.width);
        assert_eq!(hit.height, rendered.height);
        assert_eq!(hit.rgba, rendered.rgba);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn raster_hub_keeps_page_index_off_the_ui_thread() {
        let dir = std::env::temp_dir().join(format!(
            "inkarchy-pdf-hub-{}-{}",
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
            preview: false,
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
        assert!(page.image.height() >= page.image.width());

        assert!(hub.request(RasterJob {
            key: "p0".into(),
            path: path.clone(),
            page: 0,
            scale: 1.0,
            preview: false,
        }));
        assert!(hub.request(RasterJob {
            key: "p1b".into(),
            path,
            page: 1,
            scale: 1.0,
            preview: false,
        }));
        let mut first = None;
        for _ in 0..200 {
            if let Some(item) = hub.take_ready().into_iter().next() {
                first = Some(item.0);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(first.as_deref(), Some("p0"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
