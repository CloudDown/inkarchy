# Inkarchy

Native stylus notes for Omarchy, written in Rust.

![Lined page with drawing tools](docs/screens/feuille.png)

<p align="center">
  <img src="docs/screens/etagere.png" alt="Notebook shelf" width="49%">
  <img src="docs/screens/liees.png" alt="Linked pages" width="49%">
</p>

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/CloudDown/inkarchy/main/install.sh | bash
```

Uninstall: `curl -fsSL https://raw.githubusercontent.com/CloudDown/inkarchy/main/install.sh | bash -s -- --uninstall`


## Usage

```bash
inkarchy                      # open the shelf
inkarchy ~/Downloads/x.pdf    # open a PDF (or PNG, JPEG, WEBP, SVG, .txt, .md) as a notebook
inkarchy --help               # all commands and gestures
```

| Key | Action |
|---|---|
| `p` `b` `c` `h` | felt-tip, fountain pen, pencil, highlighter |
| `e` / `shift+e` | stroke eraser / area eraser |
| `l` `t` `i` | lasso, text, image |
| `[` `]` · `1-9` | thickness · ink color |
| `m` | cycle paper |
| `+` `-` · `0` | zoom · fit to screen |
| `ctrl+z` / `ctrl+y` | undo / redo |
| `ctrl+o` | open a file |
| `ctrl+s` | save |
| `ctrl+e` / `ctrl+shift+e` | export PNG / PDF |

[Docs](docs/GUIDE.md)
