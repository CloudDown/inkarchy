# Changelog

## Unreleased

### Added
- The media tool also places videos. A small on-video button plays or pauses; the still poster is exported with the note, and videos move and resize like images.

## 0.5.0 — 2026-10-20

### Added
- The ink case shows a colour column: tap the ink drop to pick a saved colour, or **+** for a mixer with a rainbow hue slider and one continuous white–colour–black slider. The check saves the custom colour; the back arrow returns to the swatches.
- Swipe along the toolbar to reach tools on a small screen. Pull a tool across the toolbar to open its storage tray and release it there to park it; the tray keeps three slots and grows only when more tools are stored, and its button sits on the toolbar while tools are parked.

### Changed
- Toolbar wells and size controls use 48 px touch targets, with compact 36 px colour swatches showing more choices in the column. Tapping the active pen again, or holding a pen or eraser, opens the four size presets and the fine slider.
- The guide no longer documents the retired sticky pad or the removed coach setup controls; it now explains the single coach switch and where its settings live.

### Removed
- The circular ink wheel; the mixer now uses two sliders.

## 0.4.0 — 2026-10-19

### Added
- Optional **exercise coach**: opt-in local Ollama or Gemini vision evaluation of handwritten maths and physics steps, shown as a qualitative meter beside the dock. Off by default and remembered per notebook. The model defaults to `qwen3-vl:4b-instruct` on a local Ollama; provider and model live in `library.json`, and Gemini keys load from the system keyring. Gemini sends page images to Google.
- Pages mode replaces the header with an on-paper edit tray: linked or separate sheets, column or row flow, two to five pages per row, paper kind, tone, and Done. A pen long press on a page enters Pages mode, and dragging a page onto a neighbour slot moves or swaps it.
- Paper tones (white, theme default, black) with matching rules, margin ink, writing ink and export. The ink case saves custom colours, and tools can be parked in the dock storage tray; both survive restarts.
- PDF pages render progressively from a quick preview to detail matched to zoom and display density, up to 432 dpi within a per-page pixel budget. Visible pages come first, off-screen queued work is discarded, and a texture LRU plus a lossless disk cache speed up revisits.
- Poppler (`pdftoppm`) renders when already installed and streams PPM pixels without temporary files; the linked hayro renderer remains the fallback, so installation adds no PDF engine dependency.
- [PDF rendering budgets and measurements](docs/PDF-PERFORMANCE.md), with an ignored `pdf_pipeline_benchmark` comparing the former and current pipeline.

### Changed
- The writing coach panel is a single **Coach this book** switch; the notebook menu and the PDF search box are tidier.
- The emoji picker decodes 40 px glyphs so its grid fills faster, while folder spine marks keep 96 px.

### Removed
- The sticky note pad and its colour wheel, large sheet, colour choices and gestures. Existing notebooks still open, and legacy sticky text stays selectable, movable and removable like other text.

### Fixed
- The long-press ring is drawn outside the fingertip instead of under it.

## 0.3.1 — 2026-10-15

### Added
- Shelf Search also matches typed text, stickies, and Unicode embedded in PDF backgrounds. A one-line excerpt appears under the spine when the hit is inside the notebook.
- Pages mode shows green `+` edge tabs and a red centre `−` to tear a page (or the sticky under that mark) without a delete picker.
- PDF pages paint a 0.25× disk-cached preview first, then a sharper pass, and open cover-fitted.
- Pushing a new `Cargo.toml` version to `main` auto-tags `v<version>` and publishes the Linux release binary when the CHANGELOG section exists.

### Changed
- Ink ribbons share strip vertices, skip the mesh on clone, and reject hit-tests outside the stroke box; the dock folds to its grip while held.
- Colour saucer and thickness taper open together from a held pen or the ink well.
- Shelf emoji sheet uses iPhone category tabs and decodes glyphs progressively so opening stays responsive.
- Shelf Search is a flat dark capsule (desk-deep fill, 1px edge, mono type) with a thin accent ring on focus.

### Removed
- Spine pin action (wheel slice, `P` shortcut, ribbon). The `pinned` field stays in saved JSON for compatibility.

### Fixed
- Shelf search excerpt no longer overlaps the notebook title.

## 0.3.0 — 2026-10-14

### Changed
- Renamed to Inkarchy. Binary `inkarchy`, window title Inkarchy, Wayland app id `com.clouddown.inkarchy`, desktop entry `inkarchy.desktop`. Notes move from `~/.local/share/omacourses` to `~/.local/share/inkarchy` on first launch (`$CAHIER_DATA` remains a deprecated alias of `$INKARCHY_DATA`, and `$CAHIER_OPEN` of `$INKARCHY_OPEN`).

## 0.2.0 — 2026-10-10

### Added
- `install.sh` installs a desktop entry (`Name=Cahier`, app id `com.clouddown.cahier`) and a scalable icon, and `--uninstall` removes them.
- `install.sh` downloads the latest Linux x86_64 release binary. `--from-source` still compiles `main`.
- `fc-match` (fontconfig) is checked, and installed on Arch when it is missing.
- GitHub Actions runs `cargo fmt`, `cargo clippy -D warnings`, and `cargo test`. Tags `v*` publish a Linux x86_64 binary on the GitHub Release.
- A failed save shows a toast instead of disappearing.
- README documents an optional Hyprland keybinding and window rule.
- Open a PDF from the command line (`cahier file.pdf`), the shelf Open button, the notebook menu, or Ctrl+O. Each page becomes a notebook page: the rendered page is a non-editable background and ink is drawn on top. The original PDF is copied into the notebook folder.
- Exporting that notebook to PDF keeps the original pages and stamps the annotations on top. Rendering uses hayro, which is compiled in, so Poppler, MuPDF, and PDFium are not required.
- Open PNG, JPEG, WEBP, and SVG as a page background. When a notebook is already open, the same files are placed on the current page.
- A short plain-text or Markdown file opens as a text block.
- The desktop entry advertises those types (`MimeType=application/pdf;image/png;image/jpeg;image/webp;image/svg+xml;` and `Exec=cahier %F`). A second `cahier file.pdf` hands the paths to the window that is already running. `install.sh` asks before running `xdg-mime default` for PDF.
- Touch gestures on the canvas: one finger pans empty paper, moves or resizes a selected image/text, and a short hold selects a stroke. Two fingers pan and zoom even with a selection.
- **Draw with finger** in the notebook menu lets the pens write by touch; erasers and lasso already work with a finger. Hold a pen or eraser button to open colour and thickness settings.
- Image import runs off the UI thread (async picker); **Replace** swaps an image without losing its placement. Dropped images land under the pointer when possible.

### Changed
- The window title is Cahier.
- Notebook and shelf saves are atomic: write a temp file in the same directory, fsync, then rename.
- Version is 0.2.0. The crate now requires Rust 1.89, which the locked dependencies already need.

### Fixed
- Empty strokes no longer panic while the ribbon is built or a shape is snapped.
- Small-window layout, with a minimum size of 320×280 (2026-10-03).
- One Download… menu with a PNG/PDF picker (2026-10-03).
- README is in English (2026-10-03).
