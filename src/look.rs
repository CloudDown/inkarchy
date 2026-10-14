//! Lectern chrome = the Omarchy / terminal palette.
//! Source: `~/.local/state/omarchy/current/theme/` (the same staging as Alacritty).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, Style, Visuals};

/// Shared serif family. Cloning the `Arc` avoids building a new name every label.
pub(crate) fn serif_family() -> FontFamily {
    static NAME: OnceLock<Arc<str>> = OnceLock::new();
    FontFamily::Name(NAME.get_or_init(|| Arc::from("serif")).clone())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeStamp {
    pub slug: String,
    pub colors_mtime: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct Look {
    pub name: String,
    pub dark: bool,
    pub desk: Color32,
    pub desk_deep: Color32,
    pub desk_edge: Color32,
    pub muted: Color32,
    pub fg: Color32,
    pub accent: Color32,
    pub paper: Color32,
    pub paper_rule: Color32,
    pub paper_rule_strong: Color32,
    pub ink: Color32,
    pub punch: Color32,
    pub shadow: Color32,
    pub inks: Vec<Color32>,
    pub highs: Vec<Color32>,
    pub cloth: Vec<Color32>,
    pub stamp: ThemeStamp,
}

impl Look {
    pub fn load() -> Self {
        let (table, slug, name, mtime) = load_omarchy_colors();
        let stamp = ThemeStamp {
            slug: slug.clone(),
            colors_mtime: mtime,
        };
        Self::from_table(table.as_ref(), name, stamp)
    }

    /// Omarchy theme change (`theme.name` plus staged `colors.toml`).
    pub fn drifted(&self) -> bool {
        let (slug, mtime) = fast_stamp();
        slug != self.stamp.slug || mtime != self.stamp.colors_mtime
    }

    fn from_table(table: Option<&toml::Table>, name: String, stamp: ThemeStamp) -> Self {
        let g = |k: &str, d: Color32| table.and_then(|t| hex_of(t, k)).unwrap_or(d);
        let mode = table
            .and_then(|t| t.get("mode").or_else(|| t.get("theme_type")))
            .and_then(|v| v.as_str())
            .unwrap_or("dark");
        let dark = mode != "light";

        let accent = g("accent", Color32::from_rgb(0x7a, 0xa2, 0xf7));
        let background = g("background", Color32::from_rgb(0x1a, 0x1b, 0x26));
        let dark_background = g("dark_background", Color32::from_rgb(0x13, 0x14, 0x1c));
        let darker = g("darker_background", Color32::from_rgb(0x0e, 0x0e, 0x14));
        let lighter = g("lighter_background", Color32::from_rgb(0x24, 0x28, 0x3b));
        let fg = g("foreground", Color32::from_rgb(0xa9, 0xb1, 0xd6));
        let muted = g("muted", Color32::from_rgb(0x41, 0x48, 0x68));
        let red = g("red", Color32::from_rgb(0xf7, 0x76, 0x8e));
        let orange = g("orange", Color32::from_rgb(0xeb, 0x92, 0x7b));
        let yellow = g("yellow", Color32::from_rgb(0xe0, 0xaf, 0x68));
        let green = g("green", Color32::from_rgb(0x9e, 0xce, 0x6a));
        let cyan = g("cyan", Color32::from_rgb(0x44, 0x9d, 0xab));
        let blue = g("blue", Color32::from_rgb(0x7a, 0xa2, 0xf7));
        let magenta = g("magenta", Color32::from_rgb(0xad, 0x8e, 0xe6));
        let brown = g("brown", Color32::from_rgb(0x75, 0x49, 0x3d));

        let paper = if dark {
            Color32::from_rgb(0xf3, 0xea, 0xd8)
        } else {
            Color32::from_rgb(0xff, 0xfc, 0xf5)
        };
        let ink = if dark {
            Color32::from_rgb(0x1c, 0x18, 0x14)
        } else {
            Color32::from_rgb(0x22, 0x1c, 0x16)
        };
        let paper_rule = mix(paper, ink, 0.16);
        let paper_rule_strong = mix(paper, ink, 0.30);
        let punch = mix(paper, ink, 0.10);

        let inks = vec![
            ink,
            mix(ink, fg, 0.35),
            red,
            orange,
            yellow,
            green,
            cyan,
            blue,
            magenta,
            brown,
            paper,
        ];
        let highs = vec![
            wash(yellow),
            wash(green),
            wash(magenta),
            wash(cyan),
            wash(orange),
        ];
        let cloth = vec![brown, red, green, blue, magenta, orange, cyan, darker]
            .into_iter()
            .map(leather)
            .collect();

        Self {
            name,
            dark,
            desk: background,
            desk_deep: dark_background,
            desk_edge: lighter,
            muted,
            fg,
            accent,
            paper,
            paper_rule,
            paper_rule_strong,
            ink,
            punch,
            shadow: Color32::from_rgba_unmultiplied(0, 0, 0, if dark { 90 } else { 40 }),
            inks,
            highs,
            cloth,
            stamp,
        }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut style = Style {
            visuals: if self.dark {
                Visuals::dark()
            } else {
                Visuals::light()
            },
            ..Style::default()
        };
        style.visuals.override_text_color = Some(self.fg);
        style.visuals.panel_fill = self.desk;
        style.visuals.window_fill = self.desk_deep;
        style.visuals.extreme_bg_color = self.desk_deep;
        style.visuals.faint_bg_color = self.desk_edge;
        style.visuals.widgets.inactive.bg_fill = self.desk_edge;
        style.visuals.widgets.hovered.bg_fill = self.muted;
        style.visuals.widgets.active.bg_fill = self.accent;
        style.visuals.widgets.inactive.corner_radius = CornerRadius::same(16);
        style.visuals.widgets.hovered.corner_radius = CornerRadius::same(16);
        style.visuals.widgets.active.corner_radius = CornerRadius::same(16);
        style.visuals.widgets.open.corner_radius = CornerRadius::same(16);
        style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(14);
        style.visuals.window_corner_radius = CornerRadius::same(20);
        style.visuals.menu_corner_radius = CornerRadius::same(16);
        style.visuals.selection.bg_fill = self.accent.gamma_multiply(0.35);
        style.visuals.hyperlink_color = self.accent;
        style.visuals.warn_fg_color = self.inks.get(4).copied().unwrap_or(self.accent);
        style.visuals.error_fg_color = self.inks.get(2).copied().unwrap_or(self.accent);
        style.visuals.window_stroke = Stroke::new(1.0_f32, self.desk_edge);
        style.visuals.window_shadow = egui::Shadow::NONE;
        style.visuals.popup_shadow = egui::Shadow::NONE;
        style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, self.fg);
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        ctx.set_style(style);
    }

    pub fn serif(&self, size: f32) -> FontId {
        FontId::new(size, serif_family())
    }

    pub fn mono(&self, size: f32) -> FontId {
        FontId::new(size, FontFamily::Monospace)
    }

    pub fn cloth_at(&self, i: u8) -> Color32 {
        self.cloth[i as usize % self.cloth.len()]
    }

    /// Theme red (ink 3 / Omarchy `red`).
    pub fn rust(&self) -> Color32 {
        self.inks.get(2).copied().unwrap_or(self.accent)
    }

    /// Theme green (Omarchy `green`).
    pub fn green(&self) -> Color32 {
        self.inks.get(5).copied().unwrap_or(self.accent)
    }

    /// Theme orange (Omarchy `orange`). Warm ochre for Open.
    pub fn orange(&self) -> Color32 {
        self.inks.get(3).copied().unwrap_or(self.accent)
    }

    /// Theme yellow (Omarchy `yellow`). Default sticky paper.
    pub fn yellow(&self) -> Color32 {
        self.inks.get(4).copied().unwrap_or(self.accent)
    }
}

pub fn current_dir() -> PathBuf {
    state_dir().join("theme")
}

fn state_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/state")
        });
    base.join("omarchy/current")
}

fn fast_stamp() -> (String, Option<SystemTime>) {
    let dir = current_dir();
    let slug = std::fs::read_to_string(state_dir().join("theme.name"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    let mtime = std::fs::metadata(dir.join("colors.toml"))
        .ok()
        .and_then(|m| m.modified().ok());
    (slug, mtime)
}

fn load_omarchy_colors() -> (Option<toml::Table>, String, String, Option<SystemTime>) {
    let staged = current_dir().join("colors.toml");
    if staged.exists() {
        let slug = std::fs::read_to_string(state_dir().join("theme.name"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "omarchy".into());
        let (table, mtime) = read_colors(&staged);
        let table = table.or_else(|| read_alacritty(&current_dir().join("alacritty.toml")));
        return (table, slug.clone(), pretty_name(&slug), mtime);
    }

    let pretty =
        stdout_trim("omarchy", &["theme", "current"]).unwrap_or_else(|| "Tokyo Night".into());
    let slug = slugify(&pretty);
    let dir = stdout_trim("omarchy", &["theme", "dir", &slug])
        .map(PathBuf::from)
        .filter(|p| p.join("colors.toml").exists())
        .or_else(|| find_theme_dir(&slug));
    let path = dir.map(|d| d.join("colors.toml"));
    let (table, mtime) = match &path {
        Some(p) if p.exists() => read_colors(p),
        _ => (None, None),
    };
    (table, slug, pretty, mtime)
}

fn read_colors(path: &Path) -> (Option<toml::Table>, Option<SystemTime>) {
    let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let Ok(text) = std::fs::read_to_string(path) else {
        return (None, mtime);
    };
    (text.parse::<toml::Table>().ok(), mtime)
}

fn read_alacritty(path: &Path) -> Option<toml::Table> {
    let text = std::fs::read_to_string(path).ok()?;
    let raw: toml::Value = text.parse().ok()?;
    let colors = raw.get("colors")?;
    let mut out = toml::Table::new();
    if let Some(p) = colors.get("primary") {
        put(&mut out, "background", p.get("background"));
        put(&mut out, "foreground", p.get("foreground"));
    }
    if let Some(n) = colors.get("normal") {
        put(&mut out, "red", n.get("red"));
        put(&mut out, "green", n.get("green"));
        put(&mut out, "yellow", n.get("yellow"));
        put(&mut out, "blue", n.get("blue"));
        put(&mut out, "magenta", n.get("magenta"));
        put(&mut out, "cyan", n.get("cyan"));
        put(&mut out, "brown", n.get("black"));
    }
    if let Some(b) = colors.get("bright") {
        put(&mut out, "muted", b.get("black"));
        put(&mut out, "light_foreground", b.get("white"));
    }
    if let Some(s) = colors.get("selection").and_then(|s| s.get("background")) {
        put(&mut out, "selection", Some(s));
        put(&mut out, "lighter_background", Some(s));
    }
    if let Some(blue) = out.get("blue").cloned() {
        out.entry("accent".to_string()).or_insert(blue);
    }
    Some(out)
}

fn put(table: &mut toml::Table, key: &str, v: Option<&toml::Value>) {
    if let Some(v) = v {
        table.insert(key.into(), v.clone());
    }
}

fn hex_of(table: &toml::Table, key: &str) -> Option<Color32> {
    parse_hex(table.get(key)?.as_str()?)
}

pub fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.trim().trim_start_matches('#');
    if s.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

fn find_theme_dir(slug: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let candidates = [
        home.join(".config/omarchy/themes").join(slug),
        PathBuf::from("/usr/share/omarchy/themes").join(slug),
        current_dir(),
    ];
    candidates
        .into_iter()
        .find(|p| p.join("colors.toml").exists())
}

fn slugify(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "-")
}

fn pretty_name(slug: &str) -> String {
    slug.split('-')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn leather(c: Color32) -> Color32 {
    let lum = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    let umber = Color32::from_rgb(0x3a, 0x27, 0x22);
    let t = if lum < 48.0 { 0.18 } else { 0.42 };
    let pulled = mix(c, umber, t);
    Color32::from_rgb(
        (pulled.r() as f32 * 0.90) as u8,
        (pulled.g() as f32 * 0.90) as u8,
        (pulled.b() as f32 * 0.90) as u8,
    )
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let u = 1.0 - t;
    Color32::from_rgb(
        (a.r() as f32 * u + b.r() as f32 * t) as u8,
        (a.g() as f32 * u + b.g() as f32 * t) as u8,
        (a.b() as f32 * u + b.b() as f32 * t) as u8,
    )
}

fn wash(c: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 90)
}

fn stdout_trim(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
