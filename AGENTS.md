# Inkarchy

Native Rust (egui) stylus notes. Product name Inkarchy. Binary `inkarchy`. One repository is this product. Local data only.

## Language
- In-app strings, README, CLI help: English. Product name stays Inkarchy.
- User often writes French; reply in French.
- Keep code identifiers and notebook JSON as they are so existing notes still open (`fab-corbeille`, `cahier-dock`, serde aliases).

## Layout of the code
- `src/app.rs` — shelf and lectern UI
- `src/ink.rs` — stroke ribbon and tools
- `src/look.rs` — Omarchy theme
- `src/document.rs` — Note / Page / sheet join
- `src/library.rs` — `library.json`, slots, trash, atomic saves
- `src/tablet.rs` — Wayland stylus and pinch
- `src/import.rs` `src/pdf.rs` `src/export.rs` `src/ipc.rs` — open files, render, export, second-instance paths
- Tests live next to the module (`src/app/tests.rs`, `#[cfg(test)]` in others). Run `cargo test --offline`.

## Theme and chrome
Chrome means the UI frame (shelf, dock, buttons, panels), not a browser. Chrome and the ink palette come from `~/.local/state/omarchy/current/theme/` (same staging as the terminal). UI mono follows `fc-match monospace`. Use `Look` colors. Paper texture is `assets/canson.jpg` (`canson-paper`).

## Keep
- Wayland proxies marked `allow(dead_code)` and empty tablet-pad Dispatch impls. Dropping them disconnects the stylus or pinch.
- Vector glyphs for chrome actions (undo/redo, text, image, open, back). Emoji atlas is for pens, lasso, pin, trash, marks.
- Saves atomic (`atomic_write`). Failed saves surface as a toast.

## Do not
- Add cloud, accounts, or a second product in this repo.
- Rename saved JSON fields or internal widget ids for style.
- Recolor Material-like cards. Each gesture is a stationery object on the lectern.

## Git
English commit messages, one concrete sentence. Update `docs/screens/` only for the feature that changed. README stays English.

## Agent context
When `graphify-out/graph.json` exists, answer codebase questions with `graphify query` before a wide search. Cold-start file hunt: caveman-explore (path:line only). Keep this file short; the graph holds the rest.
