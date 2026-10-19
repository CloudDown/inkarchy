//! Reproducible PDF pipeline measurements, deliberately outside normal tests.
//! cargo test --offline --release pdf_pipeline_benchmark -- --ignored --nocapture

use super::*;
use lopdf::dictionary;
use std::time::{Duration, Instant};

fn fixture(scan: bool) -> Vec<u8> {
    let mut doc = lopdf::Document::load_mem(&text_pdf("Benchmark")).unwrap();
    let page_id = doc.get_pages()[&1];
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("MediaBox", vec![0.into(), 0.into(), 595.into(), 842.into()]);
    let mut content = String::new();
    if scan {
        let (w, h) = (1200, 1600);
        let pixels: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let x = i % w;
                let y = i / w;
                // Scan-like paper noise, text rows and a colored diagram.
                let v = if x > 70 && x < 1100 && y % 35 < 5 {
                    40
                } else {
                    235 + (i * 17 % 20) as u8
                };
                [
                    v,
                    v.saturating_sub((x % 17) as u8),
                    v.saturating_sub((y % 13) as u8),
                ]
            })
            .collect();
        let mut image = lopdf::Stream::new(
            lopdf::dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => w,
                "Height" => h, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
            },
            pixels,
        );
        image.compress().unwrap();
        let image_id = doc.add_object(image);
        let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
        let resources = page.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
        resources.set("XObject", lopdf::dictionary! { "Scan" => image_id });
        content.push_str("q 595 0 0 842 0 0 cm /Scan Do Q\n");
    } else {
        for row in 0..85 {
            content.push_str(&format!(
                "BT /F1 7 Tf 24 {} Td (Fourier integral - small text, equations and diagrams 0123456789) Tj ET\n",
                820 - row * 9
            ));
            for col in 0..16 {
                content.push_str(&format!(
                    "0.2 0.4 0.7 RG 0.3 w {} {} 12 4 re S\n",
                    30 + col * 32,
                    817 - row * 9
                ));
            }
        }
    }
    doc.change_page_content(page_id, content.into_bytes())
        .unwrap();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

/// The former production path, including its second PNG encoding for cache.
fn legacy(path: &Path) -> RasterImage {
    let prefix = path.with_file_name("legacy");
    assert!(Command::new("pdftoppm")
        .args(["-png", "-f", "1", "-l", "1", "-r", "108"])
        .arg(path)
        .arg(&prefix)
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success());
    let png = path.with_file_name("legacy-1.png");
    let img = image::open(&png).unwrap().into_rgba8();
    std::fs::remove_file(png).unwrap();
    let rendered = Rendered {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
        scale: 1.5,
    };
    image::save_buffer(
        path.with_file_name("legacy-cache.png"),
        &rendered.rgba,
        rendered.width,
        rendered.height,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    RasterImage::from(&rendered)
}

fn median(mut f: impl FnMut() -> RasterImage) -> Duration {
    let mut samples = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        std::hint::black_box(f());
        samples.push(start.elapsed());
    }
    samples.sort();
    samples[2]
}

#[test]
#[ignore = "Manual release-mode PDF performance comparison; requires Poppler"]
fn pdf_pipeline_benchmark() {
    assert!(poppler_available(), "Install Poppler to compare pipelines");
    let dir = std::env::temp_dir().join(format!("inkarchy-pdf-bench-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, scan) in [("text-vectors", false), ("scan", true)] {
        let path = dir.join(format!("{name}.pdf"));
        std::fs::write(&path, fixture(scan)).unwrap();
        // Same-resolution output must be pixel-identical to the former path.
        let previous = legacy(&path);
        let current = render_screen_page(&path, 0, 1.5, &mut None).unwrap();
        assert_eq!(
            previous.image.pixels,
            RasterImage::from(&current).image.pixels
        );
        let old = median(|| legacy(&path));
        let new = median(|| {
            let rendered = render_screen_page(&path, 0, 1.5, &mut None).unwrap();
            let image = RasterImage::from(&rendered);
            save_raster_cache(&path, 0, &rendered);
            image
        });
        let preview =
            median(|| RasterImage::from(&render_screen_page(&path, 0, 0.5, &mut None).unwrap()));
        let detail =
            median(|| RasterImage::from(&render_screen_page(&path, 0, 3.0, &mut None).unwrap()));
        let warm = median(|| RasterImage::from(&load_raster_cache(&path, 0, 1.5).unwrap()));
        println!("{name}: median of 5, legacy 108dpi {old:.1?}; new 108dpi + cache {new:.1?}; first preview 36dpi {preview:.1?}; detail 216dpi {detail:.1?}; warm cache 108dpi {warm:.1?}; same-quality speedup {:.2}x", old.as_secs_f64() / new.as_secs_f64());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
