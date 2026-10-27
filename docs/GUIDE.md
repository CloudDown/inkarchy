# Inkarchy

Stylus notes on an **Omarchy lectern**. Inkarchy is a native Rust and egui app for handwritten notes, sketches, and diagrams.

The **shelf** holds your notebooks; each notebook opens onto a page with paper, drawing tools, and a ruler. The interface follows the active Omarchy theme, while the paper keeps its own look.

Your notes stay on your computer in `~/.local/share/inkarchy`. Writing needs no account or network connection. The optional exercise coach can use a local model or an API provider you configure. Notes from Cahier (`~/.local/share/omacourses`) are moved there on first launch.

## What's New

### Exercise coach

Click the **graduation cap** beside the toolbar. The panel has one switch, **Coach this book**; the choice is remembered for that notebook. Once on, the slim bar under the cap shows a qualitative verdict on the latest completed, readable maths or physics step: on track, check this step, possible error, or not enough information. It is an estimate, not a grade.

The coach reads the `ai` block of `~/.local/share/inkarchy/library.json`: `provider` (`"Ollama"` or `"Gemini"`), `local_url`, `local_model` and `api_model`.

- **Local · Ollama:** run Ollama on this computer and install a vision model with `ollama pull qwen3-vl:4b-instruct`. Use an explicit Instruct variant: Thinking variants may consume the short response budget before producing a verdict. The first analysis includes model loading; CPU-only inference can be too slow for continuous feedback. There is no automatic switch to a remote provider.
- **API · Gemini:** store a key with `secret-tool store --label="Inkarchy Gemini API key" application inkarchy service gemini`, then set `"provider": "Gemini"`. Enabled notebooks send the current page to Google; provider charges and data policies apply. Keys are never stored in notebook files or `library.json`.

Analysis runs in the background, starts after a short writing pause and samples continuous writing at most once every 1.2 seconds, using completed strokes. It makes no further requests while the page is unchanged. Unfinished formulas, unclear handwriting and missing statements stay neutral, and corrections invalidate the previous evaluation. Provider errors pause requests without interrupting writing.

The optional local smoke test uses a synthetic vector-ink exercise, never your notebooks:

```bash
INKARCHY_AI_TEST_MODEL=qwen3-vl:4b-instruct cargo test --offline live_local_coach_smoke -- --ignored --nocapture
```

It measures image preparation and model latency on a first and a subsequent request. Real handwriting, fractions, units, corrections and PDF exercises still need checking on the selected model; fast feedback is a target rather than a guaranteed delay.

Model selection and the limits of the current measurements are documented in [Exercise coach models](EXERCISE_COACH_MODELS.md).

### Multi-page notebooks, two ways to read them

Add pages around the current sheet. Display them edge to edge in *Linked* mode, or keep a gap between them in *Separate* mode.

<p align="center">
  <img src="screens/liees.png" alt="Linked pages, joined edge to edge" width="49%">
  <img src="screens/separees.png" alt="Separate pages with a gap" width="49%">
</p>

Click the **Edit pages** grid button at the top right to open **Pages**. The lectern becomes a workbench: pages stay in the middle, a **composition gauge** sits on one side, a **fan of paper stock** on the other.

- **Join:** two miniature sheets on the gauge. One pair touching, one pair with a gutter.
- **Arrange:** stack or a single row, then drag the column number, scroll over it, or use the `−` / `+` stepper. One drag or wheel burst is one undo step.
- **Paper:** hover a leaf in the fan to preview ruling; tap to apply to the **entire notebook**. The chips under the fan set a flat page colour (white, cream, sand, grey, black) with no Canson grain. `Ctrl+Z` undoes it.
- **Move:** long-press a page, then drag it onto a neighbour to swap or onto a free slot to move. Edges scroll; `Esc` cancels.
- **Add or remove:** tap a green `+` on a free edge to add a sheet, or the red `−` in the centre to tear one off.
- **Finish:** `Esc`, double-tap the desk, or Pages again. On a narrow window, **Arrange** and **Paper** bring one instrument forward.

<img src="screens/pages-management.png" alt="Pages workbench with a composition gauge, paper stock fan, green add tabs and a centre minus" width="100%">

<p align="center">
  <img src="screens/onglets.png" alt="Tabs for adding a page" width="80%">
</p>

### Paper for every idea

Choose lined, Cornell, grid, dotted, graph, or blank paper. Page colour is a flat fill: white, cream, sand, grey, or black. Canson grain is only on chrome (emoji atlas and help), not on the writing sheet. Cycle paper with `M`.

<p align="center">
  <img src="screens/millimetre.png" alt="Graph paper and drawing tools" width="80%">
</p>

### A shelf and a bin

Keep your notebooks on the shelf and restore any you have moved to the bin.

<p align="center">
  <img src="screens/shelf.png" alt="Notebook shelf" width="49%">
  <img src="screens/corbeille.png" alt="Notebooks in the bin" width="49%">
</p>

## Preview

![Cornell page with ink, highlighter and the tool dock](screens/hero.png)

## Run

### Install (Arch Linux / Omarchy)

```bash
curl -fsSL https://raw.githubusercontent.com/CloudDown/inkarchy/main/install.sh | bash
```

The installer checks runtime libraries, including `fc-match` from fontconfig, asks before installing any that are missing, downloads the latest release binary, and puts `inkarchy` in `~/.local/bin`. It also installs a desktop entry and icon (`inkarchy.desktop`, app id `com.clouddown.inkarchy`) so Inkarchy appears in the application menu. An older `cahier` binary and desktop entry are removed if they are present.

Compile `main` from source instead:

```bash
curl -fsSL https://raw.githubusercontent.com/CloudDown/inkarchy/main/install.sh | bash -s -- --from-source
```

Remove the binary, desktop entry, and icon:

```bash
curl -fsSL https://raw.githubusercontent.com/CloudDown/inkarchy/main/install.sh | bash -s -- --uninstall
```

Other Linux distributions work when the graphics libraries are already installed. `--from-source` also needs Rust and Cargo.

```bash
cargo run --release
inkarchy --help
inkarchy --data-dir
```

Binary: `inkarchy`. Data: `~/.local/share/inkarchy` (or `$INKARCHY_DATA`; `$CAHIER_DATA` is a deprecated alias).

```bash
CAHIER_OPEN=Mindset inkarchy   # open a notebook by title
```

Theme read from `omarchy theme current` / `colors.toml`.

## Open a file

Inkarchy opens a PDF, an image, or a short text file as a notebook.

```bash
inkarchy ~/Downloads/x.pdf
inkarchy notes.png diagram.svg readme.md
```

Each PDF page becomes a page. The rendered page stays put as the background, and ink goes on top. Downloading the notebook as PDF keeps the original page and draws the annotations over it. The original file is copied into the notebook folder.

PNG, JPEG, WEBP, and SVG become a page background when you open them from the shelf or the command line. If a notebook is already open, the same image is placed on the current page. A short `.txt` or `.md` file (up to 64 KiB of text) becomes one text block.

The media tool (`i`) places images and videos on the open page. Videos use the system `ffmpeg` and `ffprobe` (install the `ffmpeg` package); they show a still poster with a small play button. Click the button to play or pause (playback is silent), and drag the video’s corners with the Hand tool to resize it like an image. Dropping a video on an open page places it too.

Open is the sheet button at the top right of the shelf. `Ctrl+O` opens the same dialog from a notebook.

PDFs appear progressively: a quick preview is replaced by a sharp render matched to zoom and display density, up to 432 dpi within a per-page pixel budget. Existing pixels stay visible during zoom; detail is refreshed after the gesture settles. Rendering, cache decoding and pixel preparation run in the background, with visible pages prioritized and obsolete queued pages discarded. A texture LRU and a lossless disk cache speed up revisiting pages.

Rendering prefers Poppler (`pdftoppm`) when already installed, streaming pixels directly without temporary PNG files. [hayro](https://crates.io/crates/hayro) is linked into the binary as the fallback. Installation does not add a PDF engine dependency.

[PDF rendering budgets and reproducible performance measurements](PDF-PERFORMANCE.md).

### Open with Inkarchy

`install.sh` writes a desktop entry with `Exec` passing every selected file (`%F`) and:

```
MimeType=application/pdf;image/png;image/jpeg;image/webp;image/svg+xml;
```

On Omarchy, in Nautilus, right-click a PDF and choose **Open with Inkarchy**. The same path from a terminal is `inkarchy ~/Downloads/x.pdf`. If Inkarchy is already running, that command opens the file in the existing window.

The installer asks before making Inkarchy the default PDF application, and only when you are at a terminal. To do it yourself:

```bash
xdg-mime default inkarchy.desktop application/pdf
```

### Hyprland

Optional keybinding in `~/.config/hypr/hyprland.conf`:

```ini
bind = SUPER, N, exec, inkarchy
```

The window uses app id `com.clouddown.inkarchy`, the same value as `StartupWMClass` in the desktop file. Example window rule:

```ini
windowrule = float, class:^(com\.clouddown\.inkarchy)$
```

## Gestures

The lectern follows the stylus: in range, the hand pans and the nib writes. Put it down, the mouse writes again.

Choose the Hand tool (`v`) to select, move, and resize page objects with the mouse or one finger. Its corner handles resize the selection. The Lasso tool only selects a group by drawing around it. `ctrl+d` duplicates the selection and `delete` removes it. Images and videos switch to the Hand tool and are selected as soon as they are inserted. In Pages mode, tap an object’s red `−` mark to remove it.

A finger on empty paper pans the canvas; two fingers pan even when something is selected. On the trackpad, two-finger drag scrolls and two-finger pinch zooms from finger distance (one finger may stay still while the other moves in or out). If the pad ignores a resting finger, use ctrl + two-finger scroll to zoom. Three-finger swipe is left to the desktop (workspace switching). On the screen, pinch still zooms. Click the **Hand** tool and enable **Draw with finger** to use the pens by touch. Click the **Eraser** tool to choose **Stroke eraser** or **Area eraser**; hold it to adjust its size. The erasers and lasso work with a finger directly. Double-tap text to type.

The toolbar and size controls use 48 px touch targets; compact 36 px color swatches show more choices in the column. Tap the active pen again, or hold a pen or eraser, to adjust its size with four presets and a precise slider. Tap the ink drop for the color column; **+** replaces the swatches with a rainbow slider and one continuous white–color–black slider. The checkmark saves the color; the back arrow returns to the swatches. Hold a saved color, then tap the bin at the foot of the column, to remove it; theme colors stay.

Swipe along the toolbar to reach tools on a small screen. Drag one tool onto another to swap their places. Pull a tool out across the toolbar to open its storage tray, then release it there to hide it. Tap the storage button to find hidden tools; tap a stored tool or drag it back to restore it. The tray always shows nine slots, with no header, and closes only when you click outside it. Drag the dotted grip to move the toolbar. Custom colors, tool order, and stored tools are remembered across restarts. The area eraser uses the size setting; its on-screen brush stays steady, so zooming out covers more of the page.

| | Mouse | Stylus in range |
|---|---|---|
| Writes | mouse | stylus |
| Pan | space + drag, middle-click, two-finger trackpad | finger, two-finger drag |
| Eraser | right-click hold (last kind), `e` / `shift+e` | stylus button hold (release returns to ink); air double-click opens the ink wheel, triple-click opens nib sizes |
| Undo | `ctrl+z` | two-finger tap |
| Zoom | two-finger pinch (one finger may stay still), ctrl + two-finger scroll, `+` `−` | pinch |

| | |
|---|---|
| `p` `b` `c` `h` | felt-tip, fountain pen, pencil, highlighter |
| `e` / `shift+e` | stroke (whole stroke) / area (under cursor) |
| `l` `t` `i` | lasso, text, media |
| `[` `]` `1-9` | thickness, ink |
| `m` | cycle paper (blank, lined, grid, dotted, graph, Cornell) |
| corners / `0` / click on % | fit to screen |
| hold the pen still ~1s | line / arrow / triangle / square / diamond / ellipse / circle |
| `ctrl+s` | save |
| `ctrl+e` `ctrl+shift+e` | PNG, PDF |

### Shelf

| | |
|---|---|
| Right-click a spine (or hold a finger) | spine wheel — color, trash, mark |
| Wastebasket | open / close the bin row |
| Drag onto the bin (or a bin cell) | trash the notebook |
| Drag out of the bin | restore to the shelf |
| Two-finger drag on the shelf | scroll |

### Pages

| | |
|---|---|
| Pages | open the workbench: composition gauge, paper stock, `+` / `−` marks |
| Long press empty paper | open Pages manage mode |
| Tap a `+` | add a unit page on that edge |
| Tap a `−` | tear that page off. A drag pans instead, and the press that opened Pages does not count |
| Long press a page, then drag | move it into a neighbour `+` slot, or swap with another page |
| Drag / scroll the column number | choose 1–6 columns; arrow keys adjust it when focused |
| Hover / tap a paper leaf | preview / apply ruling to the entire notebook |
| Escape while dragging | cancel the move and keep Pages open |
| Escape, double-tap, or Pages again | return to writing |
| Two miniature sheets on the gauge | flush sheet vs gutter between pages |

First launch: notebooks *Mindset* and *Art direction*.
