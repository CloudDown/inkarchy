use crate::document::{squiggle, stroke_from_polyline, Note, PaperKind, TextBox, PAGE_W};
use crate::ink::{InkStroke, Nib};
use crate::library::Library;
use egui::{Color32, Pos2, Vec2};

pub fn seed_if_needed(lib: &mut Library) {
    if lib.index.seeded || !lib.index.notes.is_empty() {
        if !lib.index.seeded {
            let saved = lib.mark_seeded();
            lib.remember_save_error(saved);
        }
        return;
    }

    let mentalite = note_mentalite();
    let da = note_direction();
    let brouillon = Note::blank("Draft", 2);
    for note in [&mentalite, &da, &brouillon] {
        let saved = lib.insert_new(note);
        lib.remember_save_error(saved);
    }
    let saved = lib.mark_seeded();
    lib.remember_save_error(saved);
}

/// Mindset = a sketch of the lectern. Almost entirely ink.
fn note_mentalite() -> Note {
    let mut n = Note::blank("Mindset", 0);
    n.paper = PaperKind::Lined;
    n.pinned = false;
    let ink = Color32::from_rgb(0x1c, 0x18, 0x14);
    let mute = Color32::from_rgb(0x5a, 0x52, 0x48);
    let accent = Color32::from_rgb(0x7f, 0xbb, 0xb3);
    let high = Color32::from_rgba_unmultiplied(0xdb, 0xbc, 0x7f, 0x9a);
    let rust = Color32::from_rgb(0xe6, 0x7e, 0x80);

    n.pages[0].strokes.push(underline(88.0, 78.0, 210.0, ink));
    n.pages[0]
        .texts
        .push(label(88.0, 88.0, 28.0, "Mindset", ink));

    // Main window
    n.pages[0]
        .strokes
        .push(rect_stroke(90.0, 170.0, 280.0, 180.0, ink, 2.2));
    n.pages[0]
        .strokes
        .push(hline(100.0, 198.0, 260.0, mute, 1.4));
    n.pages[0].strokes.push(stroke_from_polyline(
        &[
            [110.0, 212.0],
            [250.0, 212.0],
            [250.0, 320.0],
            [110.0, 320.0],
            [110.0, 212.0],
        ],
        Nib::Fineliner,
        mute,
        1.2,
    ));
    for (x, c) in [(112.0, rust), (128.0, high), (144.0, accent)] {
        n.pages[0].strokes.push(dot(x, 184.0, c, 3.2));
    }

    // Accent window
    n.pages[0]
        .strokes
        .push(rect_stroke(300.0, 210.0, 220.0, 150.0, accent, 2.0));
    n.pages[0]
        .strokes
        .push(hline(310.0, 236.0, 200.0, accent, 1.3));
    n.pages[0].strokes.push(stroke_from_polyline(
        &squiggle(Pos2::new(330.0, 270.0), Pos2::new(480.0, 320.0), 6.0, 22),
        Nib::Brush,
        accent,
        3.6,
    ));

    // Small window
    n.pages[0]
        .strokes
        .push(rect_stroke(200.0, 380.0, 160.0, 100.0, mute, 1.8));
    n.pages[0]
        .strokes
        .push(hline(210.0, 402.0, 140.0, mute, 1.2));
    n.pages[0].strokes.push(stroke_from_polyline(
        &squiggle(Pos2::new(220.0, 420.0), Pos2::new(340.0, 460.0), 4.0, 14),
        Nib::Pencil,
        mute,
        1.6,
    ));

    n.pages[0].strokes.push(arrow(
        Pos2::new(400.0, 160.0),
        Pos2::new(540.0, 120.0),
        rust,
        2.6,
    ));

    // Stylus
    n.pages[0].strokes.push(stroke_from_polyline(
        &stylus(Pos2::new(600.0, 190.0), 0.48),
        Nib::Fineliner,
        ink,
        2.1,
    ));
    n.pages[0].strokes.push(stroke_from_polyline(
        &squiggle(Pos2::new(620.0, 250.0), Pos2::new(760.0, 300.0), 2.0, 12),
        Nib::Pencil,
        mute,
        1.2,
    ));

    // Finger path
    n.pages[0].strokes.push(stroke_from_polyline(
        &squiggle(Pos2::new(560.0, 360.0), Pos2::new(820.0, 400.0), 10.0, 24),
        Nib::Brush,
        accent,
        2.4,
    ));
    n.pages[0].strokes.push(arrow(
        Pos2::new(800.0, 395.0),
        Pos2::new(860.0, 410.0),
        accent,
        2.0,
    ));

    // Highlighter + stroke
    n.pages[0].strokes.push(stroke_from_polyline(
        &[[90.0, 520.0], [380.0, 528.0]],
        Nib::Highlighter,
        high,
        22.0,
    ));
    n.pages[0].strokes.push(stroke_from_polyline(
        &squiggle(Pos2::new(100.0, 540.0), Pos2::new(360.0, 560.0), 3.0, 16),
        Nib::Brush,
        ink,
        2.2,
    ));

    // Star + ribbon
    n.pages[0].strokes.push(stroke_from_polyline(
        &star(Pos2::new(980.0, 150.0), 32.0),
        Nib::Brush,
        accent,
        2.8,
    ));
    n.pages[0].strokes.push(stroke_from_polyline(
        &squiggle(Pos2::new(920.0, 210.0), Pos2::new(1050.0, 430.0), 16.0, 40),
        Nib::Brush,
        rust,
        4.5,
    ));

    // Open notebook
    n.pages[0]
        .strokes
        .push(rect_stroke(780.0, 470.0, 240.0, 170.0, ink, 2.1));
    n.pages[0]
        .strokes
        .push(vline(900.0, 478.0, 154.0, mute, 1.5));
    for y in [510.0, 540.0, 570.0, 600.0] {
        n.pages[0].strokes.push(hline(795.0, y, 95.0, mute, 1.1));
        n.pages[0].strokes.push(hline(910.0, y, 95.0, mute, 1.1));
    }
    for i in 0..5 {
        let y = 490.0 + i as f32 * 30.0;
        n.pages[0].strokes.push(dot(900.0, y, mute, 2.4));
    }
    n
}

fn note_direction() -> Note {
    let mut n = Note::blank("Art direction", 3);
    n.paper = PaperKind::Millimetre;
    n.pinned = false;
    let ink = Color32::from_rgb(0x1c, 0x18, 0x14);
    let red = Color32::from_rgb(0xe6, 0x7e, 0x80);
    let body = concat!(
        "Two families, not a Material palette.\n\n",
        "1. The lectern — Hyprland chrome, colors of the active theme (here, Omarchy colors.toml).\n",
        "2. The page — a physical object. Cream, graph paper, binder holes. Dark mode dims the desk, not the paper.\n\n",
        "Mono for the OS, serif for the notebook title.\n",
        "One scene per gesture: cloth-bound spines, a pencil case, a brass ruler, a page torn off at export.\n\n",
        "If two screens look the same when you squint, one of them has no object."
    );
    n.pages[0].texts.push(text(72.0, 64.0, 17.0, body, ink));
    n.pages[0]
        .strokes
        .push(rect_stroke(720.0, 420.0, 200.0, 120.0, red, 2.4));
    n.pages[0]
        .strokes
        .push(rect_stroke(730.0, 430.0, 80.0, 100.0, ink, 1.6));
    n.pages[0]
        .strokes
        .push(rect_stroke(820.0, 430.0, 90.0, 100.0, ink, 1.6));
    n
}

fn text(x: f32, y: f32, size: f32, s: &str, color: Color32) -> TextBox {
    let mut t = TextBox::new(Pos2::new(x, y), color);
    t.size = [PAGE_W - x - 48.0, 640.0];
    t.size_pt = size;
    t.text = s.to_string();
    t
}

fn label(x: f32, y: f32, size: f32, s: &str, color: Color32) -> TextBox {
    let mut t = TextBox::new(Pos2::new(x, y), color);
    t.size = [360.0, size * 1.6];
    t.size_pt = size;
    t.text = s.to_string();
    t
}

fn underline(x: f32, y: f32, w: f32, color: Color32) -> InkStroke {
    stroke_from_polyline(
        &squiggle(Pos2::new(x, y), Pos2::new(x + w, y + 3.0), 1.6, 18),
        Nib::Brush,
        color,
        3.2,
    )
}

fn rect_stroke(x: f32, y: f32, w: f32, h: f32, color: Color32, width: f32) -> InkStroke {
    densify(
        &[[x, y], [x + w, y], [x + w, y + h], [x, y + h], [x, y]],
        Nib::Fineliner,
        color,
        width,
    )
}

fn hline(x: f32, y: f32, w: f32, color: Color32, width: f32) -> InkStroke {
    densify(&[[x, y], [x + w, y]], Nib::Fineliner, color, width)
}

fn vline(x: f32, y: f32, h: f32, color: Color32, width: f32) -> InkStroke {
    densify(&[[x, y], [x, y + h]], Nib::Fineliner, color, width)
}

fn densify(pts: &[[f32; 2]], nib: Nib, color: Color32, width: f32) -> InkStroke {
    let mut dense = Vec::new();
    for pair in pts.windows(2) {
        for i in 0..=8 {
            let t = i as f32 / 8.0;
            dense.push([
                pair[0][0] + (pair[1][0] - pair[0][0]) * t,
                pair[0][1] + (pair[1][1] - pair[0][1]) * t,
            ]);
        }
    }
    stroke_from_polyline(&dense, nib, color, width)
}

fn dot(x: f32, y: f32, color: Color32, r: f32) -> InkStroke {
    let mut pts = Vec::new();
    for i in 0..=16 {
        let a = i as f32 / 16.0 * std::f32::consts::TAU;
        pts.push([x + a.cos() * r, y + a.sin() * r]);
    }
    stroke_from_polyline(&pts, Nib::Brush, color, r * 0.9)
}

fn arrow(from: Pos2, to: Pos2, color: Color32, width: f32) -> InkStroke {
    let dir = (to - from).normalized();
    let n = Vec2::new(-dir.y, dir.x);
    let tip = to;
    let a = tip - dir * 14.0 + n * 7.0;
    let b = tip - dir * 14.0 - n * 7.0;
    densify(
        &[
            [from.x, from.y],
            [to.x, to.y],
            [a.x, a.y],
            [to.x, to.y],
            [b.x, b.y],
        ],
        Nib::Fineliner,
        color,
        width,
    )
}

fn stylus(tip: Pos2, ang: f32) -> Vec<[f32; 2]> {
    let (s, c) = ang.sin_cos();
    let map = |x: f32, y: f32| [tip.x + x * c - y * s, tip.y + x * s + y * c];
    vec![
        map(0.0, 0.0),
        map(12.0, 4.0),
        map(18.0, 6.0),
        map(90.0, 10.0),
        map(110.0, 8.0),
        map(118.0, 0.0),
        map(110.0, -8.0),
        map(90.0, -10.0),
        map(18.0, -6.0),
        map(12.0, -4.0),
        map(0.0, 0.0),
    ]
}

fn star(c: Pos2, r: f32) -> Vec<[f32; 2]> {
    let mut pts = Vec::new();
    for i in 0..10 {
        let a = i as f32 / 10.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let rad = if i % 2 == 0 { r } else { r * 0.42 };
        pts.push([c.x + a.cos() * rad, c.y + a.sin() * rad]);
    }
    pts.push(pts[0]);
    pts
}
