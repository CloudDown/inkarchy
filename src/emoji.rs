//! Folder icons. Noto Color Emoji is a CBDT bitmap (PNG), with no outlines:
//! egui will not rasterize it. We catalog every glyph and decode on demand.
//!
//! Picker order follows the iPhone emoji keyboard categories.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;

/// Decode size for shelf spines (picker cells scale the same bitmap down).
/// 96 keeps folder marks sharp; the picker stays light via a per-frame decode budget.
const TEX_SIDE: u32 = 96;

pub struct Glyph {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// iPhone emoji keyboard tabs (Recents omitted).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EmojiGroup {
    SmileysPeople,
    AnimalsNature,
    FoodDrink,
    Activity,
    TravelPlaces,
    Objects,
    Symbols,
    Flags,
}

impl EmojiGroup {
    pub const ALL: [Self; 8] = [
        Self::SmileysPeople,
        Self::AnimalsNature,
        Self::FoodDrink,
        Self::Activity,
        Self::TravelPlaces,
        Self::Objects,
        Self::Symbols,
        Self::Flags,
    ];

    pub fn tab(self) -> char {
        match self {
            Self::SmileysPeople => '😀',
            Self::AnimalsNature => '🐻',
            Self::FoodDrink => '🍔',
            Self::Activity => '⚽',
            Self::TravelPlaces => '🚗',
            Self::Objects => '💡',
            Self::Symbols => '💜',
            Self::Flags => '🏁',
        }
    }
}

pub struct Atlas {
    font: Option<FontBytes>,
    glyphs: HashMap<char, Glyph>,
    /// Membership for `key()` — O(1).
    catalog_set: HashSet<char>,
    /// Glyphs split like the iPhone emoji tabs.
    sections: Vec<(EmojiGroup, Vec<char>)>,
}

impl Atlas {
    pub fn load() -> Self {
        let mut atlas = Self {
            font: None,
            glyphs: HashMap::new(),
            catalog_set: HashSet::new(),
            sections: Vec::new(),
        };
        let Some(data) = color_emoji_bytes() else {
            return atlas;
        };
        let Some(font) = FontBytes::parse(data) else {
            return atlas;
        };
        let chars = font.list_chars();
        atlas.sections = section_chars(&chars);
        atlas.catalog_set = atlas
            .sections
            .iter()
            .flat_map(|(_, chars)| chars.iter().copied())
            .collect();
        atlas.font = Some(font);
        atlas
    }

    #[cfg(test)]
    pub fn catalog_len(&self) -> usize {
        self.catalog_set.len()
    }

    #[cfg(test)]
    pub fn catalog(&self) -> Vec<char> {
        self.sections
            .iter()
            .flat_map(|(_, chars)| chars.iter().copied())
            .collect()
    }

    pub fn sections(&self) -> &[(EmojiGroup, Vec<char>)] {
        &self.sections
    }

    pub fn group_chars(&self, group: EmojiGroup) -> &[char] {
        self.sections
            .iter()
            .find(|(g, _)| *g == group)
            .map(|(_, chars)| chars.as_slice())
            .unwrap_or(&[])
    }

    /// Decode at most one new glyph when `budget` allows. Returns whether a
    /// decode was deferred (caller should request a repaint).
    pub fn ensure_budgeted(&mut self, ch: char, budget: &mut u32) -> (Option<&Glyph>, bool) {
        if self.glyphs.contains_key(&ch) {
            return (self.glyphs.get(&ch), false);
        }
        if *budget == 0 {
            return (None, true);
        }
        *budget -= 1;
        let Some(g) = self.font.as_ref().and_then(|font| font.decode(ch)) else {
            return (None, false);
        };
        self.glyphs.insert(ch, g);
        (self.glyphs.get(&ch), false)
    }

    #[cfg(test)]
    pub fn ensure(&mut self, ch: char) -> Option<&Glyph> {
        let mut budget = u32::MAX;
        self.ensure_budgeted(ch, &mut budget).0
    }

    pub fn key(&self, emoji: &str) -> Option<char> {
        emoji.chars().find(|ch| {
            *ch != '\u{fe0f}'
                && *ch != '\u{fe0e}'
                && *ch != '\u{200d}'
                && self.catalog_set.contains(ch)
        })
    }
}

fn section_chars(chars: &[char]) -> Vec<(EmojiGroup, Vec<char>)> {
    let mut buckets: HashMap<EmojiGroup, Vec<char>> = HashMap::new();
    for &ch in chars {
        let Some(group) = group_of(ch as u32) else {
            continue;
        };
        buckets.entry(group).or_default().push(ch);
    }
    EmojiGroup::ALL
        .into_iter()
        .filter_map(|group| {
            let mut list = buckets.remove(&group)?;
            if list.is_empty() {
                return None;
            }
            list.sort_by_key(|ch| *ch as u32);
            Some((group, list))
        })
        .collect()
}

/// Map a code point to an iPhone-style tab. `None` drops components that are
/// not useful alone (skin tones, lone regional letters, etc.).
fn group_of(cp: u32) -> Option<EmojiGroup> {
    // Skin tones and hair components — not standalone icons.
    if (0x1F3FB..=0x1F3FF).contains(&cp) || (0x1F9B0..=0x1F9B3).contains(&cp) {
        return None;
    }
    // Lone regional-indicator letters need a pair for a flag.
    if (0x1F1E6..=0x1F1FF).contains(&cp) {
        return None;
    }
    if !usable(cp) {
        return None;
    }

    // Flags (complete flag glyphs, not RI pairs).
    if matches!(cp, 0x1F3C1 | 0x1F3F3 | 0x1F3F4 | 0x1F6A9) || (0x1F38C..=0x1F38C).contains(&cp) {
        return Some(EmojiGroup::Flags);
    }

    // Smileys & Emotion + People & Body (one iPhone tab).
    if (0x1F600..=0x1F64F).contains(&cp)
        || (0x1F466..=0x1F487).contains(&cp)
        || (0x1F48B..=0x1F48B).contains(&cp)
        || (0x1F4A9..=0x1F4A9).contains(&cp)
        || (0x1F910..=0x1F93A).contains(&cp)
        || (0x1F970..=0x1F97F).contains(&cp)
        || (0x1F9B0..=0x1F9B9).contains(&cp)
        || (0x1F9C0..=0x1F9C0).contains(&cp)
        || (0x1F9D0..=0x1F9DF).contains(&cp)
        || (0x1F9E0..=0x1F9E0).contains(&cp)
        || (0x1FA70..=0x1FA7F).contains(&cp)
        || (0x1FAF0..=0x1FAFF).contains(&cp)
        || (0x1F44A..=0x1F44F).contains(&cp)
        || (0x1F450..=0x1F450).contains(&cp)
        || (0x1F590..=0x1F590).contains(&cp)
        || (0x1F595..=0x1F596).contains(&cp)
        || (0x1F90C..=0x1F90F).contains(&cp)
        || (0x1F918..=0x1F91F).contains(&cp)
        || (0x1F574..=0x1F575).contains(&cp)
        || matches!(
            cp,
            0x2639 | 0x263A | 0x1F440 | 0x1F441 | 0x1F445 | 0x1F463 | 0x1F9BE | 0x1F9BF
        )
    {
        return Some(EmojiGroup::SmileysPeople);
    }

    // Animals & Nature.
    if (0x1F400..=0x1F43F).contains(&cp)
        || (0x1F980..=0x1F9AF).contains(&cp)
        || (0x1F9A0..=0x1F9AE).contains(&cp)
        || (0x1FAB0..=0x1FABF).contains(&cp)
        || (0x1F330..=0x1F343).contains(&cp)
        || (0x1F344..=0x1F344).contains(&cp)
        || (0x1F394..=0x1F394).contains(&cp)
        || (0x1F3F5..=0x1F3F5).contains(&cp)
        || (0x1F490..=0x1F490).contains(&cp)
        || (0x2618..=0x2618).contains(&cp)
        || (0x1F4AE..=0x1F4AE).contains(&cp)
        || matches!(
            cp,
            0x1F525 | 0x2600 | 0x2601 | 0x26C5 | 0x1F308 | 0x1F30A | 0x1F30B
        )
        || (0x1F300..=0x1F321).contains(&cp)
        || (0x1F324..=0x1F32C).contains(&cp)
        || (0x26C8..=0x26C8).contains(&cp)
        || (0x1F4A5..=0x1F4A7).contains(&cp)
        || (0x1F4A8..=0x1F4A8).contains(&cp)
    {
        return Some(EmojiGroup::AnimalsNature);
    }

    // Food & Drink.
    if (0x1F345..=0x1F37F).contains(&cp)
        || (0x1F950..=0x1F96F).contains(&cp)
        || (0x1F32D..=0x1F32F).contains(&cp)
        || (0x1F9C1..=0x1F9CF).contains(&cp)
        || (0x1FAD0..=0x1FADF).contains(&cp)
        || matches!(cp, 0x2615 | 0x1F375)
    {
        return Some(EmojiGroup::FoodDrink);
    }

    // Activity (sport, games, music, arts).
    if (0x1F3A0..=0x1F3C0).contains(&cp)
        || (0x1F3C2..=0x1F3D3).contains(&cp)
        || (0x1F3BD..=0x1F3C1).contains(&cp)
        || (0x1F3C5..=0x1F3CF).contains(&cp)
        || (0x1F3D0..=0x1F3D3).contains(&cp)
        || (0x1F3F8..=0x1F3FF).contains(&cp)
        || (0x1F93A..=0x1F94F).contains(&cp)
        || (0x1F9E7..=0x1F9E9).contains(&cp)
        || (0x26BD..=0x26BE).contains(&cp)
        || (0x26F3..=0x26F3).contains(&cp)
        || (0x1F004..=0x1F004).contains(&cp)
        || (0x1F0CF..=0x1F0CF).contains(&cp)
        || (0x1FA80..=0x1FA8F).contains(&cp)
        || matches!(cp, 0x265F | 0x1F3B5 | 0x1F3B6 | 0x1F3BC)
    {
        return Some(EmojiGroup::Activity);
    }

    // Travel & Places.
    if (0x1F680..=0x1F6FF).contains(&cp)
        || (0x1F3D4..=0x1F3F0).contains(&cp)
        || (0x1F3E0..=0x1F3F0).contains(&cp)
        || (0x1F5FB..=0x1F5FF).contains(&cp)
        || (0x1F30C..=0x1F30F).contains(&cp)
        || (0x1F310..=0x1F31F).contains(&cp)
        || (0x1F5FA..=0x1F5FA).contains(&cp)
        || (0x1F9E6..=0x1F9E6).contains(&cp)
        || (0x1F9F3..=0x1F9F3).contains(&cp)
        || (0x1F6D5..=0x1F6DF).contains(&cp)
        || (0x1F6E0..=0x1F6EC).contains(&cp)
        || (0x1F6F0..=0x1F6F9).contains(&cp)
        || (0x1F6FA..=0x1F6FC).contains(&cp)
        || (0x1F9BC..=0x1F9BD).contains(&cp)
        || matches!(
            cp,
            0x26EA | 0x26F2 | 0x26FA | 0x2668 | 0x1F3A1 | 0x1F3A2 | 0x1F488 | 0x1F492
        )
        || (0x1F54B..=0x1F54E).contains(&cp)
        || (0x26E9..=0x26EA).contains(&cp)
        || (0x1F5FD..=0x1F5FF).contains(&cp)
    {
        return Some(EmojiGroup::TravelPlaces);
    }

    // Objects.
    if (0x1F4A0..=0x1F4FF).contains(&cp)
        || (0x1F500..=0x1F53D).contains(&cp)
        || (0x1F550..=0x1F567).contains(&cp)
        || (0x1F56F..=0x1F573).contains(&cp)
        || (0x1F576..=0x1F579).contains(&cp)
        || (0x1F57A..=0x1F57A).contains(&cp)
        || (0x1F587..=0x1F58A).contains(&cp)
        || (0x1F5A4..=0x1F5FF).contains(&cp)
        || (0x1F9E0..=0x1F9FF).contains(&cp)
        || (0x1FA90..=0x1FA9F).contains(&cp)
        || (0x1FAA0..=0x1FAAF).contains(&cp)
        || (0x1F321..=0x1F321).contains(&cp)
        || (0x1F39B..=0x1F39F).contains(&cp)
        || (0x1F3A7..=0x1F3A7).contains(&cp)
        || (0x1F4BB..=0x1F4CE).contains(&cp)
        || (0x231A..=0x231B).contains(&cp)
        || (0x23F0..=0x23F3).contains(&cp)
        || (0x260E..=0x260E).contains(&cp)
        || (0x1F4DE..=0x1F4DF).contains(&cp)
        || matches!(cp, 0x2709 | 0x270F | 0x2712 | 0x1F4DA | 0x1F4D4 | 0x1F4D3)
    {
        return Some(EmojiGroup::Objects);
    }

    // Symbols (hearts, signs, punctuation emoji, keycaps leftovers).
    if (0x1F300..=0x1F5FF).contains(&cp)
        || (0x2600..=0x27BF).contains(&cp)
        || (0x1F7E0..=0x1F7EB).contains(&cp)
        || (0x1F90D..=0x1F90E).contains(&cp)
        || (0x2763..=0x2764).contains(&cp)
        || (0x1F9E1..=0x1F9E1).contains(&cp)
        || (0x1FA75..=0x1FA77).contains(&cp)
        || (0x3030..=0x3030).contains(&cp)
        || (0x303D..=0x303D).contains(&cp)
        || (0x3297..=0x3299).contains(&cp)
        || (0x25AA..=0x25AB).contains(&cp)
        || (0x25B6..=0x25B6).contains(&cp)
        || (0x25C0..=0x25C0).contains(&cp)
        || (0x25FB..=0x25FE).contains(&cp)
        || (0x2B05..=0x2B07).contains(&cp)
        || (0x2B1B..=0x2B1C).contains(&cp)
        || (0x2B50..=0x2B50).contains(&cp)
        || (0x2B55..=0x2B55).contains(&cp)
        || matches!(
            cp,
            0x00A9
                | 0x00AE
                | 0x2122
                | 0x2139
                | 0x24C2
                | 0x2714
                | 0x2716
                | 0x274C
                | 0x274E
                | 0x2753
                | 0x2754
                | 0x2755
                | 0x2757
                | 0x2795
                | 0x2796
                | 0x2797
                | 0x27A1
                | 0x27B0
                | 0x27BF
        )
    {
        return Some(EmojiGroup::Symbols);
    }

    // Anything else that survived `usable` — put with Symbols so it stays reachable.
    Some(EmojiGroup::Symbols)
}

struct Group {
    start: u32,
    end: u32,
    glyph: u32,
}

struct FontBytes {
    data: Vec<u8>,
    groups: Vec<Group>,
    cblc: (usize, usize),
    cbdt: (usize, usize),
}

impl FontBytes {
    fn parse(data: Vec<u8>) -> Option<Self> {
        let tables = tables(&data)?;
        let cmap = slice(&data, *tables.get("cmap")?)?;
        let cblc = *tables.get("CBLC")?;
        let cbdt = *tables.get("CBDT")?;
        Some(Self {
            groups: cmap_groups(cmap)?,
            data,
            cblc,
            cbdt,
        })
    }

    fn cblc(&self) -> &[u8] {
        &self.data[self.cblc.0..self.cblc.0 + self.cblc.1]
    }

    fn cbdt(&self) -> &[u8] {
        &self.data[self.cbdt.0..self.cbdt.0 + self.cbdt.1]
    }

    fn list_chars(&self) -> Vec<char> {
        let mut out = Vec::new();
        for g in &self.groups {
            for cp in g.start..=g.end {
                if !usable(cp) {
                    continue;
                }
                let Some(ch) = char::from_u32(cp) else {
                    continue;
                };
                let Some(gid) = glyph_id(&self.groups, cp) else {
                    continue;
                };
                if locate(self.cblc(), gid).is_some() {
                    out.push(ch);
                }
            }
        }
        out
    }

    fn decode(&self, ch: char) -> Option<Glyph> {
        let gid = glyph_id(&self.groups, ch as u32)?;
        let (off, len) = locate(self.cblc(), gid)?;
        let blob = self.cbdt().get(off..off + len)?;
        let dlen = read_u32(blob, 5)? as usize;
        let png = blob.get(9..9 + dlen)?;
        let img = image::load_from_memory(png).ok()?.into_rgba8();
        let (w, h) = img.dimensions();
        let img = if w > TEX_SIDE || h > TEX_SIDE {
            let s = (TEX_SIDE as f32 / w.max(h) as f32).min(1.0);
            let nw = ((w as f32 * s).round() as u32).max(1);
            let nh = ((h as f32 * s).round() as u32).max(1);
            image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
        } else {
            img
        };
        Some(Glyph {
            width: img.width() as usize,
            height: img.height() as usize,
            rgba: img.into_raw(),
        })
    }
}

fn usable(cp: u32) -> bool {
    // Variation selectors, ZWJ, tags — not icons on their own.
    if matches!(cp, 0x200D | 0xFE0E | 0xFE0F | 0x20E3) {
        return false;
    }
    if (0xE0020..=0xE007F).contains(&cp) {
        return false;
    }
    // Private use / lone regional tags are useless in the grid.
    if (0xE000..=0xF8FF).contains(&cp) {
        return false;
    }
    // High enough for symbols, or a dingbat / emoji block.
    cp >= 0x00A9 || (0x203C..=0x3299).contains(&cp)
}

fn color_emoji_bytes() -> Option<Vec<u8>> {
    for p in [
        "/usr/share/fonts/noto/NotoColorEmoji.ttf",
        "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    ] {
        if let Ok(b) = std::fs::read(p) {
            return Some(b);
        }
    }
    let out = Command::new("fc-match")
        .args(["Noto Color Emoji", "-f", "%{file}\n"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() {
        None
    } else {
        std::fs::read(PathBuf::from(p)).ok()
    }
}

fn tables(data: &[u8]) -> Option<HashMap<&str, (usize, usize)>> {
    if data.len() < 12 || data.get(0..4) != Some(&[0, 1, 0, 0]) {
        return None;
    }
    let n = read_u16(data, 4)? as usize;
    let mut map = HashMap::new();
    for i in 0..n {
        let at = 12 + i * 16;
        let tag = std::str::from_utf8(data.get(at..at + 4)?).ok()?;
        let off = read_u32(data, at + 8)? as usize;
        let len = read_u32(data, at + 12)? as usize;
        map.insert(tag, (off, len));
    }
    Some(map)
}

fn slice(data: &[u8], span: (usize, usize)) -> Option<&[u8]> {
    data.get(span.0..span.0 + span.1)
}

fn cmap_groups(cmap: &[u8]) -> Option<Vec<Group>> {
    let n = read_u16(cmap, 2)? as usize;
    for i in 0..n {
        let at = 4 + i * 8;
        let sub = read_u32(cmap, at + 4)? as usize;
        if read_u16(cmap, sub)? != 12 {
            continue;
        }
        let ng = read_u32(cmap, sub + 12)? as usize;
        let mut groups = Vec::with_capacity(ng);
        for g in 0..ng {
            let o = sub + 16 + g * 12;
            groups.push(Group {
                start: read_u32(cmap, o)?,
                end: read_u32(cmap, o + 4)?,
                glyph: read_u32(cmap, o + 8)?,
            });
        }
        return Some(groups);
    }
    None
}

fn glyph_id(groups: &[Group], cp: u32) -> Option<u16> {
    let i = groups.partition_point(|g| g.end < cp);
    let g = groups.get(i)?;
    if cp < g.start || cp > g.end {
        return None;
    }
    u16::try_from(g.glyph + (cp - g.start)).ok()
}

fn locate(cblc: &[u8], gid: u16) -> Option<(usize, usize)> {
    let nsub = read_u32(cblc, 16)? as usize;
    let arr = read_u32(cblc, 8)? as usize;
    for i in 0..nsub {
        let ent = arr + i * 8;
        let first = read_u16(cblc, ent)?;
        let last = read_u16(cblc, ent + 2)?;
        if gid < first || gid > last {
            continue;
        }
        let base = arr + read_u32(cblc, ent + 4)? as usize;
        if read_u16(cblc, base)? != 1 || read_u16(cblc, base + 2)? != 17 {
            return None;
        }
        let image_off = read_u32(cblc, base + 4)? as usize;
        let idx = (gid - first) as usize;
        let o1 = read_u32(cblc, base + 8 + idx * 4)? as usize;
        let o2 = read_u32(cblc, base + 12 + idx * 4)? as usize;
        if o2 < o1 {
            return None;
        }
        return Some((image_off + o1, o2 - o1));
    }
    None
}

fn read_u16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_emoji_catalog() {
        let mut atlas = Atlas::load();
        if atlas.catalog_len() == 0 {
            return;
        }
        assert!(atlas.catalog_len() > 200);
        let key = atlas.key("📝").expect("memo key");
        let g = atlas.ensure(key).expect("memo");
        assert!(g.width >= 24 && g.height >= 24);
    }

    #[test]
    fn toolbar_emojis_decode() {
        let mut atlas = Atlas::load();
        if atlas.catalog_len() == 0 {
            return;
        }
        for em in ["🖊️", "🖌️", "✏️", "🖍️", "🧼", "🧽", "🪢", "🗑️"] {
            let key = atlas.key(em).unwrap_or_else(|| panic!("missing key {em}"));
            let g = atlas.ensure(key).unwrap_or_else(|| panic!("missing {em}"));
            assert!(g.width >= 24 && g.height >= 24, "{em}");
        }
    }

    #[test]
    fn sections_follow_iphone_tab_order() {
        let atlas = Atlas::load();
        if atlas.catalog_len() == 0 {
            return;
        }
        let groups: Vec<_> = atlas.sections().iter().map(|(g, _)| *g).collect();
        let mut expected = Vec::new();
        for g in EmojiGroup::ALL {
            if atlas.group_chars(g).is_empty() {
                continue;
            }
            expected.push(g);
        }
        assert_eq!(groups, expected);
        assert_eq!(groups.first().copied(), Some(EmojiGroup::SmileysPeople));
        let smile = atlas.group_chars(EmojiGroup::SmileysPeople);
        assert!(smile.contains(&'😀') || smile.iter().any(|c| (*c as u32) >= 0x1F600));
        // Regional-indicator letters stay out of the grid.
        assert!(!atlas
            .catalog()
            .iter()
            .any(|c| (0x1F1E6..=0x1F1FF).contains(&(*c as u32))));
    }

    #[test]
    fn ensure_budgeted_defers_when_empty() {
        let mut atlas = Atlas::load();
        if atlas.catalog_len() == 0 {
            return;
        }
        let ch = *atlas.catalog().last().expect("glyph");
        let mut budget = 0;
        let (g, deferred) = atlas.ensure_budgeted(ch, &mut budget);
        assert!(g.is_none());
        assert!(deferred);
        budget = 1;
        let (g, deferred) = atlas.ensure_budgeted(ch, &mut budget);
        assert!(g.is_some());
        assert!(!deferred);
        assert_eq!(budget, 0);
    }
}
