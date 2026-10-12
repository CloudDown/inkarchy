//! PDF pages, rendered with [hayro](https://crates.io/crates/hayro).
//!
//! hayro is pure Rust and is linked into the binary. Cahier does not need
//! Poppler, MuPDF, or PDFium on the system.

use std::path::Path;
use std::sync::Arc;

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
pub fn render_scale(zoom: f32, pixels_per_point: f32) -> f32 {
    let want = (zoom.max(0.05) * pixels_per_point.max(0.5) * PAPER_PER_PT).clamp(0.75, 4.0);
    const STEPS: [f32; 4] = [1.5, 2.0, 3.0, 4.0];
    STEPS
        .iter()
        .copied()
        .find(|step| *step + 0.05 >= want)
        .unwrap_or(4.0)
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
    let pdf = open_pdf(bytes.to_vec())?;
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
    })
}

fn clamp_scale(pt_w: f32, pt_h: f32, scale: f32) -> f32 {
    let scale = scale.clamp(0.25, 4.0);
    let long = pt_w.max(pt_h).max(1.0) * scale;
    if long > 4096.0 {
        (scale * (4096.0 / long)).max(0.25)
    } else {
        scale
    }
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
        assert_eq!(render_scale(0.5, 1.0), 1.5);
        assert!(render_scale(3.0, 1.0) >= 3.0);
    }
}
