# Inkarchy

Stylus notes on an **Omarchy lectern**. Inkarchy is a native Rust and egui app for handwritten notes, sketches, and diagrams.

The **shelf** holds your notebooks; each notebook opens onto a page with paper, drawing tools, and a ruler. The interface follows the active Omarchy theme, while the paper keeps its own look.

Your notes stay on your computer in `~/.local/share/inkarchy`. Inkarchy needs no account or network connection. Notes from Cahier (`~/.local/share/omacourses`) are moved there on first launch.

## What's New

### Multi-page notebooks, two ways to read them

Add pages around the current sheet. Display them edge to edge in *Linked* mode, or keep a gap between them in *Separate* mode.

<p align="center">
  <img src="screens/liees.png" alt="Linked pages, joined edge to edge" width="49%">
  <img src="screens/separees.png" alt="Separate pages with a gap" width="49%">
</p>

Use the `+` tabs to add a page along any free edge. The top-right button hides them.

<p align="center">
  <img src="screens/onglets.png" alt="Tabs for adding a page" width="80%">
</p>

### Paper for every idea

Choose lined, Cornell, grid, dotted, graph, or blank paper. Canson paper adds a subtle texture to notes and sketches. Cycle paper with `M`.

<p align="center">
  <img src="screens/millimetre.png" alt="Graph paper and drawing tools" width="80%">
</p>

### Lectern objects

A pin on the shelf is a ribbon on the spine. Drag the sticky pad onto the page; hold a sticky for its color.

<p align="center">
  <img src="screens/sticky-notes.png" alt="Sticky notes on a dotted page" width="80%">
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

Open is the sheet button at the top right of the shelf, and a row in the notebook menu. `Ctrl+O` opens the same dialog.

PDF rendering uses [hayro](https://crates.io/crates/hayro), linked into the `inkarchy` binary. Install does not add Poppler, MuPDF, or PDFium.

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

On a touch screen, drag an image or a text box directly with one finger, then use its corner handles to resize it. A short hold on a handwritten stroke selects it; the selection offers labelled editing, duplication and deletion actions. Images are selected as soon as they are inserted, and **Replace** changes an image without losing its placement.

A finger on empty paper pans the canvas; two fingers pan and zoom even when something is selected. Choose **Draw with finger** in the notebook menu to use the pens by touch. The erasers and lasso work with a finger directly. Tap text to select it and choose **Edit text** (or double-tap it) to type. Hold a pen/eraser button to open its colour and thickness settings; tapping the active pen opens these settings too.

| | Mouse | Stylus in range |
|---|---|---|
| Writes | mouse | stylus |
| Pan | space + drag, middle-click, trackpad | finger, two-finger drag |
| Eraser | right-click hold (last kind), `e` / `shift+e` | stylus button hold, air-click to toggle |
| Undo | `ctrl+z` | two-finger tap |
| Zoom | pinch, two-finger trackpad, `+` `−` | pinch |

| | |
|---|---|
| `p` `b` `c` `h` | felt-tip, fountain pen, pencil, highlighter |
| `e` / `shift+e` | stroke (whole stroke) / area (under cursor) |
| `l` `t` `i` | lasso, text, image |
| `[` `]` `1-9` | thickness, ink |
| `m` | cycle paper (blank, lined, grid, dotted, graph, slate) |
| corners / `0` / click on % | fit to screen |
| hold the pen still ~1s | line / arrow / triangle / square / diamond / ellipse / circle |
| `ctrl+s` | save |
| `ctrl+e` `ctrl+shift+e` | PNG, PDF |

### Shelf

| | |
|---|---|
| Right-click a spine (or hold a finger) | spine wheel — pin, color, trash, mark |
| Wastebasket | open / close the bin row |
| Drag onto the bin (or a bin cell) | trash the notebook |
| Drag out of the bin | restore to the shelf |
| Two-finger drag on the shelf | scroll |

### Pages

| | |
|---|---|
| `+` on a free edge | add a unit page there |
| Top-right page-tabs button | hide or show those `+` tabs |
| Fold corner (when more than one unit) | tear that unit off |
| ⋯ → Linked / Separate | flush sheet vs gutter between pages |

First launch: notebooks *Mindset* and *Art direction*.
