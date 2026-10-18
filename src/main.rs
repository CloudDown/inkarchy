mod ai;
mod app;
mod camera;
mod cli;
mod document;
mod emoji;
mod export;
mod fonts;
mod import;
mod ink;
mod ipc;
mod library;
mod look;
mod pdf;
mod pressure;
mod seed;
mod tablet;
mod undo;

use app::InkarchyApp;

const HELP: &str = "\
inkarchy — stylus notes, Omarchy lectern

Usage:
  inkarchy
  inkarchy --help
  inkarchy --version
  inkarchy --data-dir
  inkarchy --theme

Opens the lectern. Notebooks live in $INKARCHY_DATA
or ~/.local/share/inkarchy ($CAHIER_DATA is a deprecated alias).
INKARCHY_OPEN opens a notebook by title ($CAHIER_OPEN is a deprecated alias).
Chrome follows ~/.local/state/omarchy/current/theme
(same palette as the terminal).

Examples:
  inkarchy
  INKARCHY_DATA=/tmp/inkarchy inkarchy
  INKARCHY_OPEN=Mindset inkarchy
  inkarchy --theme
  inkarchy --data-dir

Gestures (inside a notebook):
  p felt-tip   b fountain   c pencil   h highlighter
  e stroke eraser   shift+e area eraser
  l lasso   t text   i image
  [ ] thickness   1-9 ink   m paper
  + − zoom   0 / corners = fit to screen
  mouse writes · space pans · right-click = last eraser
  stylus in range: stylus writes · finger pans · two-finger tap = undo
  trackpad: pinch or two fingers (vertical) = zoom
  screen pinch / ctrl+scroll zoom
  stylus button = last eraser (hold)   air-click = pen ↔ eraser
  2nd button = lasso (hold)
  drag the pencil-case handle → top / bottom / sides
  hold the pen still ~1s ≈ line, arrow, triangle, square, diamond, ellipse, circle
  drag the sticky pad onto the page   double-tap sticky = draw   Text tool = type   drag corner = size   hold = color
  delete button = pick objects or pages   drag a zone like the shelf   the list checks them   Delete removes the checked ones   Escape closes
  ctrl+z/y   ctrl+s save   ctrl+e png   ctrl+shift+e pdf
  ctrl+o open a PDF, image, or short text file

Open a file (a new window, or the one already running):
  inkarchy ~/Downloads/x.pdf
  inkarchy notes.png diagram.svg readme.md
";

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let files = match cli::parse_args(&args) {
        Ok(cli::CliRequest::Help) => {
            print!("{HELP}");
            return Ok(());
        }
        Ok(cli::CliRequest::Version) => {
            println!("inkarchy {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Ok(cli::CliRequest::DataDir) => {
            println!("{}", crate::library::data_dir().display());
            return Ok(());
        }
        Ok(cli::CliRequest::Theme) => {
            let look = crate::look::Look::load();
            println!("theme: {}", look.name.to_lowercase());
            println!(
                "file:  {}",
                crate::look::current_dir().join("colors.toml").display()
            );
            println!("desk:  {}", hex(look.desk));
            println!("accent:{}", hex(look.accent));
            println!("ink:   {}", hex(look.fg));
            if let Some(font) = crate::fonts::mono_file() {
                println!("mono:  {}", font.display());
            }
            return Ok(());
        }
        Ok(cli::CliRequest::Run(files)) => files,
        Err(err) => {
            eprintln!("Error: {err}.");
            eprintln!("  inkarchy --help");
            std::process::exit(2);
        }
    };
    if !files.is_empty() && ipc::forward(&files) {
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([320.0, 280.0])
            .with_title("Inkarchy")
            .with_app_id("com.clouddown.inkarchy"),
        vsync: false,
        ..Default::default()
    };
    eframe::run_native(
        "Inkarchy",
        options,
        Box::new(move |cc| Ok(Box::new(InkarchyApp::new(cc, files)))),
    )
}

fn hex(c: egui::Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}
