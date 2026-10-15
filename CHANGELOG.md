# Changelog

## 0.3.1 — 2026-10-15

### Added
- Shelf Search also matches typed text, stickies, and Unicode embedded in PDF backgrounds. A one-line excerpt appears under the spine when the hit is inside the notebook.
- Pages mode shows green `+` edge tabs and a red centre `−` to tear a page (or the sticky under that mark) without a delete picker.
- PDF pages paint a 0.25× disk-cached preview first, then a sharper pass, and open cover-fitted.
- Pushing a new `Cargo.toml` version to `main` auto-tags `v<version>` and publishes the Linux release binary when the CHANGELOG section exists.

### Changed
- Ink ribbons share strip vertices, skip the mesh on clone, and reject hit-tests outside the stroke box; the dock folds to its grip while held.
- Colour saucer and thickness taper open together from a held pen or the ink well.

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
