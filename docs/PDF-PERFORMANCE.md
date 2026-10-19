# PDF rendering performance

## Pipeline

- First paint uses a small preview (at most 36 dpi). A full-resolution cache hit bypasses the preview.
- Detail follows the page's physical screen size, including HiDPI scaling. Resolution levels reach 432 dpi, limited to 4096 pixels per side and 8 megapixels per page. For A4, the pixel budget limits detail to approximately 294 dpi.
- During zoom, the existing texture stays visible. Detail requests wait for 120 ms of stable zoom to avoid rendering every pinch increment.
- A worker handles PDF parsing, rendering, disk-cache decoding and conversion to egui pixels. It requests a repaint when pixels are ready. The UI uploads at most one PDF texture per frame.
- Blank visible pages take priority over detail upgrades. Off-screen queued work is discarded. The queue holds at most 64 requests and two completed images; a render already in progress can finish.
- Poppler streams lossless PPM pixels through stdout instead of writing and decoding a temporary PNG. Hayro remains the built-in fallback and reuses the last parsed document.
- The texture LRU targets 96 MiB, protecting visible pages. The budget is soft while visible pages need more space. Previews and reading resolutions up to 144 dpi use a fast, lossless, atomically written PNG cache; higher zoom levels remain memory-only. Cache identity includes source filename, size, modification time, page and resolution.

## Reproduce the comparison

```bash
cargo test --offline --release pdf_pipeline_benchmark -- --ignored --nocapture
```

The benchmark generates two A4 documents: dense small text/vector diagrams and a synthetic scanned image. It compares the former PNG-based pipeline with the new pipeline at the same 108 dpi, including cache writing and egui pixel preparation. It also verifies pixel-identical output at that resolution. Preview, detail and warm-cache measurements include pixel preparation but exclude GPU upload and UI scheduling.

Local results with Poppler 26.08.0, optimized build, median of five runs:

| Fixture | Former pipeline, 108 dpi | New pipeline + cache, 108 dpi | Speedup | Preview, 36 dpi | Detail, 216 dpi | Warm cache, 108 dpi |
|---|---:|---:|---:|---:|---:|---:|
| Text and vectors | 227.2 ms | 35.8 ms | 6.34× | 22.5 ms | 76.1 ms | 3.2 ms |
| Synthetic scan | 100.8 ms | 51.1 ms | 1.97× | 33.2 ms | 114.0 ms | 6.8 ms |

These are pipeline measurements on generated fixtures, not guarantees for arbitrary PDFs or end-to-end opening times. The benchmark does not flush the OS file cache. It reparses the PDF for each cold render; the interactive worker retains its parsed document.
