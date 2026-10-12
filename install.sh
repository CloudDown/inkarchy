#!/usr/bin/env bash
set -euo pipefail

REPO="CloudDown/omacourses"
SOURCE_URL="https://github.com/${REPO}/archive/refs/heads/main.tar.gz"
RELEASE_ASSET="cahier-x86_64-unknown-linux-gnu"
RAW_ROOT="https://raw.githubusercontent.com/${REPO}/main"
APP_ID="com.clouddown.cahier"

fail() {
  printf 'install.sh: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Install Cahier, a stylus notes app.

Usage:
  install.sh                 download the latest Linux x86_64 release
  install.sh --from-source   compile the main branch with Cargo
  install.sh --uninstall     remove the binary, desktop entry, and icon
  install.sh --help

The binary goes to ${CARGO_INSTALL_ROOT:-$HOME/.local}/bin. A desktop entry
and icon are installed under ~/.local/share so Cahier shows up in the menu.
EOF
}

command -v tar >/dev/null 2>&1 || fail "tar is required."
if command -v curl >/dev/null 2>&1; then
  download() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
  download() { wget -qO- "$1"; }
else
  fail "Install curl or wget, then run this script again."
fi

script_dir=""
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

mode="release"
case "${1:-}" in
  "" ) ;;
  --from-source) mode="source" ;;
  --uninstall) mode="uninstall" ;;
  -h|--help) usage; exit 0 ;;
  *) fail "Unknown option: $1 (try --help)" ;;
esac

install_dir="${CARGO_INSTALL_ROOT:-$HOME/.local}/bin"
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
desktop_path="${data_home}/applications/${APP_ID}.desktop"
icon_dir="${data_home}/icons/hicolor/scalable/apps"
icon_path="${icon_dir}/${APP_ID}.svg"

fetch_repo_file() {
  local rel="$1"
  local dest="$2"
  if [[ -n "$script_dir" && -f "${script_dir}/${rel}" ]]; then
    install -m 644 "${script_dir}/${rel}" "$dest"
    return
  fi
  download "${RAW_ROOT}/${rel}" > "$dest"
  chmod 644 "$dest"
}

install_desktop() {
  local bin_path="$1"
  mkdir -p "$(dirname "$desktop_path")" "$icon_dir"
  fetch_repo_file "assets/${APP_ID}.svg" "$icon_path"
  {
    printf '%s\n' \
      '[Desktop Entry]' \
      'Type=Application' \
      'Name=Cahier' \
      'Comment=Stylus notes on an Omarchy lectern' \
      "Exec=\"${bin_path}\" %F" \
      "Icon=${APP_ID}" \
      'Terminal=false' \
      'MimeType=application/pdf;image/png;image/jpeg;image/webp;image/svg+xml;' \
      'Categories=Office;Education;' \
      "StartupWMClass=${APP_ID}"
  } > "$desktop_path"
  chmod 644 "$desktop_path"
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$(dirname "$desktop_path")" >/dev/null 2>&1 || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "${data_home}/icons/hicolor" >/dev/null 2>&1 || true
  fi
}

uninstall_cahier() {
  rm -f "${install_dir}/cahier" "$desktop_path" "$icon_path"
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$(dirname "$desktop_path")" >/dev/null 2>&1 || true
  fi
  printf 'Removed Cahier from %s, and the desktop entry and icon.\n' "$install_dir"
}

if [[ "$mode" == "uninstall" ]]; then
  uninstall_cahier
  exit 0
fi

# PDF pages are rendered by hayro, compiled into the binary. No Poppler, MuPDF, or PDFium package.
runtime_packages=(wayland libxkbcommon libx11 libxcursor libxrandr libxi fontconfig)
build_packages=(base-devel pkgconf rust "${runtime_packages[@]}")
if [[ "$mode" == "source" ]]; then
  packages=("${build_packages[@]}")
else
  packages=("${runtime_packages[@]}")
fi

if command -v pacman >/dev/null 2>&1 && [[ -r /etc/os-release ]] && grep -Eiq '^(ID|ID_LIKE)=.*(arch|omarchy)' /etc/os-release; then
  if ! pacman -T "${packages[@]}" >/dev/null 2>&1; then
    printf 'Cahier needs system libraries. Install them with pacman? [y/N] '
    [[ -t 0 ]] || fail "Missing Arch dependencies; run interactively to install them."
    read -r answer
    [[ "$answer" =~ ^[Yy]$ ]] || fail "Required dependencies were not installed."
    if (( EUID == 0 )); then
      pacman -S --needed "${packages[@]}"
    elif command -v sudo >/dev/null 2>&1; then
      sudo pacman -S --needed "${packages[@]}"
    else
      fail "Install these packages as root: ${packages[*]}"
    fi
  fi
fi

command -v fc-match >/dev/null 2>&1 || fail "fc-match is required. Install the fontconfig package, then run this script again."

install_binary() {
  local src="$1"
  mkdir -p "$install_dir"
  install -m 755 "$src" "${install_dir}/cahier"
  install_desktop "${install_dir}/cahier"
  printf '\nInstalled Cahier to %s/cahier\n' "$install_dir"
  printf 'Desktop entry: %s\n' "$desktop_path"
  printf 'Icon: %s\n' "$icon_path"
  if [[ ":$PATH:" != *":$install_dir:"* ]]; then
    printf 'Add it to your PATH if needed: export PATH="%s:$PATH"\n' "$install_dir"
  fi
  printf 'Run it with: cahier\n'
  offer_pdf_default
}

# Ask before changing the PDF handler. A non-interactive install only prints the command.
offer_pdf_default() {
  if ! command -v xdg-mime >/dev/null 2>&1; then
    return
  fi
  local cmd="xdg-mime default ${APP_ID}.desktop application/pdf"
  if [[ -t 0 ]]; then
    printf 'Open PDF files with Cahier by default? [y/N] '
    local answer=""
    read -r answer || answer=""
    if [[ "$answer" =~ ^[Yy]$ ]]; then
      xdg-mime default "${APP_ID}.desktop" application/pdf || true
    fi
  else
    printf 'To open PDFs with Cahier by default, run:\n  %s\n' "$cmd"
  fi
}

if [[ "$mode" == "source" ]]; then
  command -v cargo >/dev/null 2>&1 || fail "Rust/Cargo is required. On Arch, install it with: sudo pacman -S rust"
  workdir="$(mktemp -d)"
  trap 'rm -rf "$workdir"' EXIT
  printf 'Downloading Cahier source…\n'
  download "$SOURCE_URL" | tar -xz -C "$workdir" --strip-components=1
  printf 'Building Cahier (first build can take a few minutes)…\n'
  (
    cd "$workdir"
    cargo build --locked --release --bin cahier
  )
  install_binary "${workdir}/target/release/cahier"
  exit 0
fi

[[ "$(uname -m)" == "x86_64" ]] || fail "No prebuilt binary for $(uname -m). Re-run with --from-source."

release_url="https://github.com/${REPO}/releases/latest/download/${RELEASE_ASSET}"
bin_tmp="$(mktemp)"
trap 'rm -f "$bin_tmp"' EXIT
printf 'Downloading the latest Cahier release…\n'
if ! download "$release_url" > "$bin_tmp"; then
  fail "Could not download the latest release. Re-run with --from-source to compile main."
fi
magic="$(od -An -t x1 -N 4 "$bin_tmp" | tr -d ' \n')"
if [[ "$magic" != "7f454c46" ]]; then
  fail "The latest release has no Linux x86_64 binary yet. Re-run with --from-source to compile main."
fi
install_binary "$bin_tmp"
