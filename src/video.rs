//! Video placed on a page: probe, poster, and a small ffmpeg-backed player.
//!
//! Inkarchy keeps videos in the notebook's `media/` folder next to images.
//! Frames are decoded with the system `ffmpeg` binary so the app stays free of
//! codec dependencies; `ffprobe` supplies the display size and duration.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

/// Extensions the media tool accepts as video. Everything else stays on the
/// image path (`import::is_placeable_image`).
pub const EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv", "avi", "ogv"];

/// Decode at most this many pixels on the long edge. The note shows a video at
/// roughly the same size as a placed image, so full camera resolution would
/// only cost bandwidth and memory.
const MAX_EDGE: u32 = 960;

/// Frames are resampled to the source rate, capped here. A steady rate keeps
/// the pipe protocol simple and positions exact without parsing PTS.
const MAX_FPS: f64 = 30.0;

pub fn extension(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    EXTENSIONS.contains(&ext.as_str()).then_some(ext)
}

pub fn is_video_path(path: &Path) -> bool {
    extension(path).is_some()
}

pub fn is_video_file(file: &str) -> bool {
    is_video_path(Path::new(file))
}

/// Poster file kept beside the video (`clip.mp4` → `clip.poster.png`).
pub fn poster_name(video_file: &str) -> String {
    let stem = Path::new(video_file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    format!("{stem}.poster.png")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meta {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub fps: f64,
}

/// Read just the display size, duration, and frame rate.
pub fn probe(path: &Path) -> Option<Meta> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,avg_frame_rate,r_frame_rate:stream_tags=rotate:stream_side_data:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let stream = root.get("streams")?.get(0)?;
    let mut width = stream.get("width")?.as_u64()? as u32;
    let mut height = stream.get("height")?.as_u64()? as u32;
    if width == 0 || height == 0 {
        return None;
    }
    if rotation(stream).abs() % 180.0 > 45.0 {
        std::mem::swap(&mut width, &mut height);
    }
    let duration = root
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0)
        .max(0.0);
    let fps = rate(stream.get("avg_frame_rate"))
        .or_else(|| rate(stream.get("r_frame_rate")))
        .unwrap_or(MAX_FPS)
        .clamp(1.0, MAX_FPS);
    Some(Meta {
        width,
        height,
        duration,
        fps,
    })
}

fn rotation(stream: &serde_json::Value) -> f64 {
    if let Some(angle) = stream
        .get("tags")
        .and_then(|t| t.get("rotate"))
        .and_then(|r| r.as_str())
        .and_then(|r| r.parse::<f64>().ok())
    {
        return angle;
    }
    stream
        .get("side_data_list")
        .and_then(|list| list.as_array())
        .and_then(|list| {
            list.iter()
                .find_map(|item| item.get("rotation").and_then(|r| r.as_f64()))
        })
        .unwrap_or(0.0)
}

fn rate(value: Option<&serde_json::Value>) -> Option<f64> {
    let text = value?.as_str()?;
    let (num, den) = text.split_once('/')?;
    let num: f64 = num.parse().ok()?;
    let den: f64 = den.parse().ok()?;
    (den > 0.0 && num > 0.0).then_some(num / den)
}

/// Pixel size used by the frame pipe and poster.
fn decode_size(width: u32, height: u32) -> (u32, u32) {
    let width = width.max(1);
    let height = height.max(1);
    let scale = (MAX_EDGE as f32 / width.max(height) as f32).min(1.0);
    let even = |v: u32| (v & !1).max(2);
    (
        even((width as f32 * scale).round() as u32),
        even((height as f32 * scale).round() as u32),
    )
}

fn poster_size(meta: &Meta) -> (u32, u32) {
    decode_size(meta.width, meta.height)
}

/// PNG poster for the still frame shown while the video is paused and in
/// exports. `None` when ffmpeg cannot read the stream.
pub fn poster_png(path: &Path, meta: &Meta) -> Option<Vec<u8>> {
    let (width, height) = poster_size(meta);
    let seek = if meta.duration > 0.5 {
        (meta.duration * 0.05).min(2.0)
    } else {
        0.0
    };
    let mut command = Command::new("ffmpeg");
    command.args(["-nostdin", "-v", "error"]);
    if seek > 0.0 {
        command.arg("-ss").arg(format!("{seek:.3}"));
    }
    let out = command
        .arg("-i")
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-an",
            "-sn",
            "-dn",
            "-vf",
            &format!("scale={width}:{height}"),
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    Some(out.stdout)
}

pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub position: f64,
}

enum Event {
    Frame(Frame),
    End,
}

/// One decoded video. Playback is a paused poster until `play` starts an
/// ffmpeg child; pause kills the child and keeps the last decoded frame.
pub struct Player {
    path: PathBuf,
    meta: Meta,
    frame_w: u32,
    frame_h: u32,
    position: f64,
    playing: bool,
    ended: bool,
    child: Option<Child>,
    events: Option<Receiver<Event>>,
}

impl Player {
    pub fn load(path: PathBuf) -> Option<Self> {
        let meta = probe(&path)?;
        Some(Self::new(path, meta))
    }

    pub fn new(path: PathBuf, meta: Meta) -> Self {
        let (frame_w, frame_h) = decode_size(meta.width, meta.height);
        Self {
            path,
            meta,
            frame_w,
            frame_h,
            position: 0.0,
            playing: false,
            ended: false,
            child: None,
            events: None,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// 0..1 for the progress line; zero when the duration is unknown.
    pub fn progress(&self) -> f32 {
        if self.meta.duration <= 0.0 {
            0.0
        } else {
            (self.position / self.meta.duration).clamp(0.0, 1.0) as f32
        }
    }

    pub fn play(&mut self) -> bool {
        if self.playing {
            return true;
        }
        if self.ended || (self.meta.duration > 0.0 && self.position >= self.meta.duration - 0.05) {
            self.position = 0.0;
            self.ended = false;
        }
        self.start()
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.stop();
    }

    /// Take every frame decoded since the last call. The UI uploads the last
    /// one; intermediate frames are already stale when the event loop is slow.
    pub fn poll(&mut self) -> Option<Frame> {
        let mut latest = None;
        let mut ended = false;
        let mut disconnected = false;
        if let Some(events) = &self.events {
            loop {
                match events.try_recv() {
                    Ok(Event::Frame(frame)) => {
                        self.position = frame.position;
                        latest = Some(frame);
                    }
                    Ok(Event::End) => {
                        ended = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if ended || disconnected {
            self.playing = false;
            self.ended = true;
            self.events = None;
            self.stop();
            if self.meta.duration > 0.0 {
                self.position = self.meta.duration;
            }
        }
        latest
    }

    fn start(&mut self) -> bool {
        self.stop();
        self.ended = false;
        let start = self.position.max(0.0);
        let (width, height, fps) = (self.frame_w, self.frame_h, self.meta.fps);
        let duration = self.meta.duration;
        let (tx, rx) = mpsc::sync_channel(2);
        let path = self.path.clone();
        let mut command = Command::new("ffmpeg");
        command.args(["-nostdin", "-v", "error", "-re"]);
        if start > 0.001 {
            command.arg("-ss").arg(format!("{start:.3}"));
        }
        command
            .arg("-i")
            .arg(&path)
            .args([
                "-an",
                "-sn",
                "-dn",
                "-r",
                &format!("{fps:.6}"),
                "-vf",
                &format!("scale={width}:{height}"),
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let Ok(mut child) = command.spawn() else {
            return false;
        };
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            return false;
        };
        let frame_bytes = width as usize * height as usize * 4;
        let spawned = thread::Builder::new()
            .name("inkarchy-video".into())
            .spawn(move || {
                let mut index = 0u64;
                loop {
                    let mut rgba = vec![0u8; frame_bytes];
                    if stdout.read_exact(&mut rgba).is_err() {
                        break;
                    }
                    let position = start + index as f64 / fps;
                    if tx
                        .send(Event::Frame(Frame {
                            rgba,
                            width,
                            height,
                            position,
                        }))
                        .is_err()
                    {
                        return;
                    }
                    index += 1;
                    if duration > 0.0 && position >= duration + 0.25 {
                        break;
                    }
                }
                let _ = tx.send(Event::End);
            });
        if spawned.is_err() {
            let _ = child.kill();
            return false;
        }
        self.child = Some(child);
        self.events = Some(rx);
        self.playing = true;
        true
    }

    fn stop(&mut self) {
        self.events = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn tool_available(name: &str) -> bool {
        Command::new(name)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[test]
    fn video_extensions_are_case_insensitive_and_leave_images_alone() {
        assert!(is_video_file("clip.mp4"));
        assert!(is_video_file("CLIP.MKV"));
        assert!(!is_video_file("photo.png"));
        assert_eq!(extension(Path::new("a/b.MOV")).as_deref(), Some("mov"));
    }

    #[test]
    fn poster_sits_beside_the_video() {
        assert_eq!(poster_name("abc.mp4"), "abc.poster.png");
        assert_eq!(poster_name("abc.webm"), "abc.poster.png");
    }

    #[test]
    fn frame_size_caps_the_long_edge_and_stays_even() {
        assert_eq!(decode_size(320, 240), (320, 240));
        assert_eq!(decode_size(3840, 2160), (960, 540));
        assert_eq!(decode_size(1080, 1920), (540, 960));
    }

    #[test]
    fn frame_rates_parse_fractions() {
        assert_eq!(
            rate(Some(&serde_json::json!("30000/1001"))),
            Some(30000.0 / 1001.0)
        );
        assert_eq!(rate(Some(&serde_json::json!("0/0"))), None);
    }

    #[test]
    fn probes_decodes_and_pauses_a_generated_clip() {
        if !tool_available("ffmpeg") || !tool_available("ffprobe") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("inkarchy-video-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip.mp4");
        let made = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=160x120:rate=10",
                "-t",
                "1",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());

        let meta = probe(&path).expect("ffprobe reads the clip");
        assert_eq!((meta.width, meta.height), (160, 120));
        assert!((meta.duration - 1.0).abs() < 0.2);
        assert!((meta.fps - 10.0).abs() < 0.1);

        let poster = poster_png(&path, &meta).expect("ffmpeg makes a poster");
        let image = image::load_from_memory(&poster).unwrap();
        assert_eq!(image.width(), 160);

        let mut player = Player::new(path, meta);
        assert!(player.play());
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut frames = 0;
        while frames < 2 && Instant::now() < deadline {
            if player.poll().is_some() {
                frames += 1;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(frames >= 2, "the frame pipe delivers decoded pixels");
        player.pause();
        assert!(!player.is_playing());
        let _ = std::fs::remove_dir_all(dir);
    }
}
