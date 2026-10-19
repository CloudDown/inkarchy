//! Optional exercise coaching. Documents never contain credentials or AI output.
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::document::Note;
use crate::look::Look;

mod credentials;
mod provider;
pub use credentials::{CredentialAction, CredentialJob};
#[cfg(test)]
pub use provider::check_connection;
#[cfg(test)]
mod tests;

pub const QUIET: Duration = Duration::from_millis(350);
pub const INTERVAL: Duration = Duration::from_millis(1200);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    Ollama,
    Gemini,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub provider: Provider,
    pub local_url: String,
    pub local_model: String,
    pub api_model: String,
    pub enabled_notebooks: HashSet<Uuid>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: Provider::Ollama,
            local_url: "http://127.0.0.1:11434".into(),
            local_model: "qwen3-vl:4b-instruct".into(),
            api_model: "gemini-3.5-flash-lite".into(),
            enabled_notebooks: HashSet::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    OnTrack,
    CheckStep,
    LikelyError,
    Insufficient,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::OnTrack => "On track",
            Self::CheckStep => "Check this step",
            Self::LikelyError => "Possible error",
            Self::Insufficient => "Not enough information",
        }
    }
    pub fn position(self) -> f32 {
        match self {
            Self::OnTrack => 0.85,
            Self::CheckStep => 0.35,
            Self::LikelyError => 0.15,
            Self::Insufficient => 0.5,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Analysis {
    pub exercise_found: bool,
    pub readable: bool,
    pub complete_step: bool,
    pub verdict: Verdict,
    pub statement: String,
    pub recognized_step: String,
    pub explanation: String,
    /// Exercise rectangle in normalized full-page coordinates, x/y/width/height.
    pub region: Option<[f32; 4]>,
}

impl Analysis {
    pub fn validate(mut self) -> Result<Self, String> {
        if self.statement.len() > 4000
            || self.recognized_step.len() > 2000
            || self.explanation.len() > 1200
        {
            return Err("The model returned an oversized answer. Try again.".into());
        }
        if let Some([x, y, w, h]) = self.region {
            if ![x, y, w, h].iter().all(|v| v.is_finite())
                || x < 0.0
                || y < 0.0
                || w <= 0.0
                || h <= 0.0
                || x + w > 1.001
                || y + h > 1.001
            {
                return Err("The model returned an invalid exercise region.".into());
            }
        }
        // An incomplete/ambiguous reading must never produce a negative judgement.
        if !self.exercise_found
            || !self.readable
            || !self.complete_step
            || self.statement.trim().is_empty()
            || self.recognized_step.trim().is_empty()
        {
            self.verdict = Verdict::Insufficient;
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub epoch: u64,
    pub revision: u64,
}

/// One running snapshot; newer edits are coalesced without cloning a queued page.
pub struct Session {
    pub note: Option<Uuid>,
    pub page: usize,
    pub reference: Option<(i32, i32)>,
    pub result: Option<Analysis>,
    pub error: Option<String>,
    pub timings: Option<Timings>,
    pub revision: u64,
    pub evaluated: u64,
    pub busy: Option<Ticket>,
    epoch: Arc<AtomicU64>,
    changed_at: Instant,
    pending_since: Instant,
    last_sent: Option<Instant>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            note: None,
            page: 0,
            reference: None,
            result: None,
            error: None,
            timings: None,
            revision: 0,
            evaluated: 0,
            busy: None,
            epoch: Arc::new(AtomicU64::new(0)),
            changed_at: Instant::now(),
            pending_since: Instant::now(),
            last_sent: None,
        }
    }
}

impl Session {
    pub fn reset(&mut self, note: Option<Uuid>) {
        self.note = note;
        self.page = 0;
        self.reference = None;
        self.invalidate(Instant::now());
        self.last_sent = None;
    }
    pub fn invalidate(&mut self, now: Instant) {
        self.result = None;
        self.error = None;
        self.timings = None;
        self.changed(false, now);
    }
    pub fn changed(&mut self, append: bool, now: Instant) {
        if self.revision == self.evaluated || !append {
            self.pending_since = now;
        }
        if !append {
            self.epoch.fetch_add(1, Ordering::Relaxed);
            self.result = None;
        }
        self.revision += 1;
        self.changed_at = now;
    }
    pub fn ready(&self, now: Instant) -> bool {
        let quiet = now.saturating_duration_since(self.changed_at) >= QUIET;
        let due = self
            .last_sent
            .is_none_or(|t| now.saturating_duration_since(t) >= INTERVAL);
        // Continuous handwriting can be sampled at the cadence using finished strokes.
        let continuous = now.saturating_duration_since(self.pending_since) >= INTERVAL;
        self.note.is_some()
            && self.busy.is_none()
            && self.error.is_none()
            && self.revision > self.evaluated
            && due
            && (quiet || continuous)
    }
    pub fn ticket(&self) -> Ticket {
        Ticket {
            epoch: self.epoch.load(Ordering::Relaxed),
            revision: self.revision,
        }
    }
    pub fn sent(&mut self, now: Instant) {
        self.busy = Some(self.ticket());
        self.last_sent = Some(now);
    }
    pub fn receive(&mut self, done: Completed) {
        if self.busy != Some(done.ticket) {
            return;
        }
        self.busy = None;
        if done.ticket.epoch != self.epoch.load(Ordering::Relaxed) {
            return;
        }
        self.evaluated = done.ticket.revision;
        self.timings = Some(done.timings);
        match done.result {
            Ok(result) => {
                self.result = Some(result);
                self.error = None;
            }
            Err(error) => {
                self.result = None;
                self.error = Some(error);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Timings {
    pub load: Duration,
    pub prepare: Duration,
    pub inference: Duration,
    pub total: Duration,
}

pub struct Request {
    pub ticket: Ticket,
    pub note: Note,
    pub reference: Option<Note>,
    pub note_dir: PathBuf,
    pub look: Look,
    pub context: Option<Analysis>,
    pub settings: AiSettings,
    pub key: String,
    pub created: Instant,
}

pub struct Completed {
    pub ticket: Ticket,
    pub result: Result<Analysis, String>,
    pub timings: Timings,
}

/// Clone just one page and omit the GPU tessellation cache.
pub fn snapshot(note: &Note, page: usize) -> Option<Note> {
    let page = note.pages.get(page)?;
    let mut copy = Note::blank("Exercise", note.cover);
    copy.id = note.id;
    copy.paper = note.paper;
    copy.paper_tone = note.paper_tone;
    copy.page_w = note.page_w;
    copy.page_h = note.page_h;
    let mut page_copy = crate::document::Page {
        texts: page.texts.clone(),
        images: page.images.clone(),
        background: page.background.clone(),
        ..Default::default()
    };
    page_copy.strokes = page
        .strokes
        .iter()
        .map(|s| crate::ink::InkStroke {
            id: s.id,
            nib: s.nib,
            color: s.color,
            width: s.width,
            points: s.points.clone(),
            sticky: s.sticky,
            mesh: None,
        })
        .collect();
    copy.pages = vec![page_copy];
    Some(copy)
}

pub struct Worker {
    tx: mpsc::SyncSender<Request>,
    rx: mpsc::Receiver<Completed>,
}

impl Worker {
    pub fn new(session: &Session, ctx: &egui::Context) -> Result<Self, String> {
        let (tx, jobs) = mpsc::sync_channel::<Request>(1);
        let (results, rx) = mpsc::channel();
        let epoch = session.epoch.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("inkarchy-coach".into())
            .spawn(move || {
                let client = provider::Transport::new();
                let mut checked_local = None;
                while let Ok(job) = jobs.recv() {
                    let mut timings = Timings::default();
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if job.ticket.epoch != epoch.load(Ordering::Relaxed) {
                            return Err("Analysis cancelled.".into());
                        }
                        if job.settings.provider == Provider::Ollama
                            && checked_local.as_ref() != Some(&job.settings)
                        {
                            let load = Instant::now();
                            provider::check_connection(&job.settings, "")?;
                            client.as_ref().map_err(Clone::clone)?.warm(
                                &job.settings,
                                &epoch,
                                job.ticket.epoch,
                            )?;
                            timings.load = load.elapsed();
                            checked_local = Some(job.settings.clone());
                        }
                        let start = Instant::now();
                        let prepared = prepare_images(&job)?;
                        timings.prepare = start.elapsed();
                        if job.ticket.epoch != epoch.load(Ordering::Relaxed) {
                            return Err("Analysis cancelled.".into());
                        }
                        let start = Instant::now();
                        let result = match &client {
                            Ok(client) => client.analyze(
                                &job.settings,
                                &job.key,
                                &prepared.images,
                                job.context.as_ref(),
                                &prepared.detail,
                                &epoch,
                                job.ticket.epoch,
                            ),
                            Err(error) => Err(error.clone()),
                        };
                        timings.inference = start.elapsed();
                        result.map(|result| prepared.restore_region(result))
                    }))
                    .unwrap_or_else(|_| Err("Could not prepare this page for analysis.".into()));
                    timings.total = job.created.elapsed();
                    if results
                        .send(Completed {
                            ticket: job.ticket,
                            result,
                            timings,
                        })
                        .is_err()
                    {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .map_err(|_| "Could not start exercise analysis.".to_owned())?;
        Ok(Self { tx, rx })
    }
    pub fn send(&self, request: Request) -> Result<(), String> {
        self.tx
            .try_send(request)
            .map_err(|_| "Exercise analysis is unavailable. Reopen the notebook to retry.".into())
    }
    pub fn poll(&self) -> Option<Completed> {
        self.rx.try_recv().ok()
    }
}

fn render(note: &Note, job: &Request) -> Result<tiny_skia::Pixmap, String> {
    let media = crate::export::MediaLoader {
        root: job.note_dir.join("media"),
    };
    // Do not silently evaluate a page whose exercise image failed to load.
    for image in &note.pages[0].images {
        if image::open(media.root.join(&image.file)).is_err() {
            return Err("An image on this page could not be read.".into());
        }
    }
    if let Some(bg) = &note.pages[0].background {
        if !job.note_dir.join(&bg.file).is_file() {
            return Err("The exercise background is missing.".into());
        }
    }
    let longest = if job.settings.provider == Provider::Ollama {
        1024.0
    } else {
        1600.0
    };
    let scale = longest / note.page_w.max(note.page_h).max(1.0);
    crate::export::raster_page(
        note,
        0,
        &job.look,
        scale,
        &media,
        &job.note_dir,
        crate::export::RasterMode::Analysis,
    )
    .ok_or_else(|| "Could not render this page.".into())
}

struct Prepared {
    images: Vec<Vec<u8>>,
    detail: String,
    viewport: [f32; 4],
}

impl Prepared {
    fn restore_region(&self, mut result: Analysis) -> Analysis {
        if let Some([x, y, w, h]) = result.region {
            let [vx, vy, vw, vh] = self.viewport;
            result.region = Some([vx + x * vw, vy + y * vh, w * vw, h * vh]);
        }
        result
    }
}

fn prepare_local(pm: &tiny_skia::Pixmap, job: &Request) -> Result<Prepared, String> {
    use egui::{pos2, vec2, Pos2, Rect};
    let page = &job.note.pages[0];
    let mut rect = Rect::NOTHING;
    if let Some([x, y, w, h]) = job.context.as_ref().and_then(|c| c.region) {
        rect = Rect::from_min_size(
            pos2(x * job.note.page_w, y * job.note.page_h),
            vec2(w * job.note.page_w, h * job.note.page_h),
        );
        for stroke in page.strokes.iter().rev().take(12) {
            if let Some((a, b)) = stroke.bbox() {
                rect = rect.union(Rect::from_min_max(a, b));
            }
        }
    } else if page.background.is_none() {
        // Empty paper margins carry no information. Retain ALL content on the first pass.
        for stroke in &page.strokes {
            if let Some((a, b)) = stroke.bbox() {
                rect = rect.union(Rect::from_min_max(a, b));
            }
        }
        for text in &page.texts {
            rect = rect.union(text.rect());
        }
        for image in &page.images {
            rect = rect.union(image.rect());
        }
    }
    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(pm.width() as f32, pm.height() as f32));
    let scale = pm.width() as f32 / job.note.page_w;
    rect = if rect.is_finite() {
        Rect::from_min_max(rect.min * scale, rect.max * scale)
            .expand(32.0)
            .intersect(bounds)
    } else {
        bounds
    };
    let x = rect.left().floor().max(0.0) as u32;
    let y = rect.top().floor().max(0.0) as u32;
    let w = (rect.right().ceil() as u32)
        .min(pm.width())
        .saturating_sub(x);
    let h = (rect.bottom().ceil() as u32)
        .min(pm.height())
        .saturating_sub(y);
    let crop = tiny_skia::IntRect::from_xywh(x as i32, y as i32, w, h)
        .and_then(|r| pm.clone_rect(r))
        .ok_or_else(|| "There is no readable content on this page.".to_owned())?;
    let viewport = [
        x as f32 / pm.width() as f32,
        y as f32 / pm.height() as f32,
        w as f32 / pm.width() as f32,
        h as f32 / pm.height() as f32,
    ];
    let mut detail =
        "Image 1 is the analysis view of the current page. Coordinates refer to this view. "
            .to_owned();
    if let Some(pt) = page.strokes.last().and_then(|s| s.points.last()) {
        detail.push_str(&format!(
            "Latest pen position in this view: ({:.3}, {:.3}). ",
            (pt.x / job.note.page_w - viewport[0]) / viewport[2],
            (pt.y / job.note.page_h - viewport[1]) / viewport[3]
        ));
    }
    let mut images = vec![resize_png(&crop, 768)?];
    if let Some(reference) = &job.reference {
        images.push(resize_png(&render(reference, job)?, 1024)?);
        detail.push_str("Image 2 is the user-selected statement reference page. ");
    }
    Ok(Prepared {
        images,
        detail,
        viewport,
    })
}

fn prepare_images(job: &Request) -> Result<Prepared, String> {
    let pm = render(&job.note, job)?;
    if job.settings.provider == Provider::Ollama {
        return prepare_local(&pm, job);
    }
    let mut images = Vec::new();
    let png = |pm: &tiny_skia::Pixmap| {
        crate::export::pixmap_png(pm).ok_or_else(|| "Could not encode this page.".to_owned())
    };
    // Always retain an overview: the writer may have started another exercise.
    if let Some(context) = &job.context {
        if let Some(region) = context.region {
            let mut rect = egui::Rect::from_min_size(
                egui::pos2(
                    region[0] * pm.width() as f32,
                    region[1] * pm.height() as f32,
                ),
                egui::vec2(
                    region[2] * pm.width() as f32,
                    region[3] * pm.height() as f32,
                ),
            );
            let scale = pm.width() as f32 / job.note.page_w;
            for stroke in job.note.pages[0].strokes.iter().rev().take(12) {
                if let Some((a, b)) = stroke.bbox() {
                    rect = rect.union(egui::Rect::from_min_max(a * scale, b * scale));
                }
            }
            let bounds = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(pm.width() as f32, pm.height() as f32),
            );
            rect = rect.expand(32.0).intersect(bounds);
            if let Some(rect) = tiny_skia::IntRect::from_xywh(
                rect.min.x as i32,
                rect.min.y as i32,
                rect.width().ceil() as u32,
                rect.height().ceil() as u32,
            ) {
                if let Some(crop) = pm.clone_rect(rect) {
                    images.push(resize_png(
                        &crop,
                        if job.settings.provider == Provider::Ollama {
                            768
                        } else {
                            1024
                        },
                    )?);
                }
            }
        }
    }
    let has_crop = !images.is_empty();
    let mut detail = String::new();
    if let Some(point) = job.note.pages[0]
        .strokes
        .last()
        .and_then(|s| s.points.last())
    {
        detail = format!("The latest pen position is ({:.3}, {:.3}) in normalized page coordinates. Evaluate the exercise near that position. ", point.x / job.note.page_w, point.y / job.note.page_h);
    }
    // Once localized, a small overview catches changes of exercise while the crop
    // preserves legible maths. Coordinates still refer to the full page.
    let overview = if has_crop {
        resize_png(
            &pm,
            if job.settings.provider == Provider::Ollama {
                576
            } else {
                768
            },
        )?
    } else {
        png(&pm)?
    };
    images.insert(0, overview);
    if has_crop {
        detail.push_str("Image 2 is a close-up of the response area. ");
    }
    if let Some(reference) = &job.reference {
        images.push(png(&render(reference, job)?)?);
        detail.push_str(&format!(
            "Image {} is the user-selected statement reference page. ",
            images.len()
        ));
    }
    Ok(Prepared {
        images,
        detail,
        viewport: [0.0, 0.0, 1.0, 1.0],
    })
}

fn resize_png(pm: &tiny_skia::Pixmap, longest: u32) -> Result<Vec<u8>, String> {
    let img = image::RgbaImage::from_raw(pm.width(), pm.height(), pm.data().to_vec())
        .ok_or_else(|| "Could not encode the page overview.".to_owned())?;
    let mut out = std::io::Cursor::new(Vec::new());
    let img = image::DynamicImage::ImageRgba8(img);
    let img = if img.width().max(img.height()) > longest {
        img.resize(longest, longest, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|_| "Could not encode the page overview.".to_owned())?;
    Ok(out.into_inner())
}
