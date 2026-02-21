#!/usr/bin/env sh
set -eu

REPO="RIZAmohammadkhan/TerminalMusicPlayer"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*\n" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing dependency: $1"; }

need curl
need tar
need uname

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux) ;;
  *) die "unsupported OS: $OS (this installer is for Linux)" ;;
esac

case "$ARCH" in
  x86_64|amd64) TARGET_TRIPLE="x86_64-unknown-linux-gnu" ;;
  *) die "unsupported arch: $ARCH (only x86_64 is supported by current releases)" ;;
esac

: "${PREFIX:=}"

if [ -n "$PREFIX" ]; then
  INSTALL_DIR="$PREFIX/bin"
elif [ "$(id -u 2>/dev/null || echo 1)" -eq 0 ]; then
  INSTALL_DIR="/usr/local/bin"
else
  INSTALL_DIR="$HOME/.local/bin"
fi

mkdir -p "$INSTALL_DIR"

API_URL="https://api.github.com/repos/${REPO}/releases/latest"
say "Fetching latest release metadata from $REPO..."

json="$(curl -fsSL "$API_URL")" || die "failed to fetch release metadata"

# Pick the first asset matching the current target triple and .tar.xz
asset_url="$(
  printf '%s' "$json" \
    | sed -n 's/.*"browser_download_url"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | grep -E "${TARGET_TRIPLE}.*\\.tar\\.xz$" \
    | head -n 1
)"

[ -n "$asset_url" ] || die "could not find a .tar.xz asset for ${TARGET_TRIPLE} in latest release"

sha_url="$(
  printf '%s' "$json" \
    | sed -n 's/.*"browser_download_url"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | grep -E "${TARGET_TRIPLE}.*\\.tar\\.xz\\.sha256$" \
    | head -n 1 || true
)"

tmpdir="$(mktemp -d)"
cleanup() { rm -rf "$tmpdir"; }
trap cleanup EXIT INT TERM

archive="$tmpdir/release.tar.xz"
say "Downloading: $asset_url"
curl -fL --retry 3 --retry-delay 1 -o "$archive" "$asset_url" || die "download failed"

if [ -n "$sha_url" ]; then
  need sha256sum
  sums="$tmpdir/release.sha256"
  say "Downloading checksums: $sha_url"
  curl -fL --retry 3 --retry-delay 1 -o "$sums" "$sha_url" || die "checksum download failed"

  expected="$(sed -n 's/^\([0-9a-fA-F]\{64\}\).*/\1/p' "$sums" | head -n 1)"
  [ -n "$expected" ] || die "could not parse sha256 from checksum file"

  actual="$(sha256sum "$archive" | awk '{print $1}')"
  [ "$expected" = "$actual" ] || die "checksum mismatch"

  say "Checksum OK"
fi

say "Extracting..."
tar -xJf "$archive" -C "$tmpdir"

# Find the trix executable within the extracted tree.
# cargo-dist archives usually contain a top-level directory.
trix_path="$(find "$tmpdir" -type f -name trix -perm -u+x 2>/dev/null | head -n 1 || true)"
[ -n "$trix_path" ] || die "could not find 'trix' executable in archive"

say "Installing to: $INSTALL_DIR/trix"
install -m 0755 "$trix_path" "$INSTALL_DIR/trix"

# yt-dlp is required for the YouTube download feature.
if ! command -v yt-dlp >/dev/null 2>&1; then
  say "Installing yt-dlp..."
  sudo curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp
  sudo chmod a+rx /usr/local/bin/yt-dlp
  say "yt-dlp installed."
fi

say "Done. Run: trix"

if ! command -v trix >/dev/null 2>&1; then
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
      say "Note: '$INSTALL_DIR' is not on your PATH. Add this to your shell rc:" 
      say "  export PATH=\"$INSTALL_DIR:\$PATH\""
      ;;
  esac
fi
