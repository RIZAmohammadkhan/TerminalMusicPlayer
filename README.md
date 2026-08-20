# Trix Player

<div align="center">

![Trix Version](https://img.shields.io/github/v/release/RIZAmohammadkhan/TerminalMusicPlayer?label=version)
![AUR Version](https://img.shields.io/aur/version/trix-player)
![License](https://img.shields.io/github/license/RIZAmohammadkhan/TerminalMusicPlayer)
![Build Status](https://img.shields.io/github/actions/workflow/status/RIZAmohammadkhan/TerminalMusicPlayer/release.yml)

**A beautiful, keyboard-driven terminal music player for Linux.**

[Features](#-features) • [Installation](#-installation) • [Usage](#-usage) • [Keybindings](#-keyboard-controls) • [Configuration](#-configuration) • [Building](#-building-from-source)

</div>

---

**Trix** is a lightweight, high-performance music player built with Rust. It is designed for Linux users who prefer the terminal, offering a clean TUI (Terminal User Interface), robust vim-like keyboard navigation, gapless playback, synced lyrics, favorites, YouTube downloads, and full MPRIS2 integration — all with a minimal resource footprint.

## 🎬 Demo

https://github.com/user-attachments/assets/4a7be924-2580-49d4-bd89-cddd8a084f2b

## ✨ Features

### Playback Engine
* **Gapless playback** — the next track is decoded and pre-loaded in the background while the current one plays, so transitions are seamless.
* **Low-latency audio** — a dedicated decode worker keeps ~750ms of audio buffered to absorb transient stalls; playback stops instantly on exit.
* **Error-tolerant decoding** — corrupt or "almost-valid" files are handled gracefully by skipping bad frames instead of failing.
* **Accurate seeking** — Symphonia container-level seeking avoids re-decoding from the start, so seeks are fast even mid-track.
* **Shuffle & Loop** — Fisher–Yates shuffle order and per-track loop (repeats seamlessly without gaps).

### Audio Formats
MP3, FLAC, WAV, OGG, M4A, AAC — decoded via `symphonia` with a `rodio` fallback decoder.

### Volume Control
* Native **ALSA system mixer** control (Master/PCM/Speaker/Headphone/Front/Line Out, auto-detected).
* Automatic **per-app gain** fallback when no usable system mixer is available.
* The active backend is shown in the UI title bar (e.g. `System (ALSA)` or `App gain`).

### Library
* **Automatic discovery** — recursively scans your XDG music directory (via `~/.config/user-dirs.dirs`), falling back to `~/Music`/`~/music`, then the current directory.
* Play a specific **directory or file** by passing it as an argument.
* **Live search** — type to filter; the selection jumps to the first matching track as you type.
* **Move to timestamp** — jump anywhere in a track by typing `1:30` or `01:02:03`.
* **Favorites** — mark tracks with a heart, toggle a favorites-only view, persisted across sessions to `~/.config/trix/favorites.txt`.
* **Delete tracks** straight from the player (double-press `D` to confirm; the library list is updated instantly).
* **Virtualized list** — only visible rows are rendered, so even huge libraries stay fast.

### Synced Lyrics
* Loads sidecar `.lrc` files automatically (`track.lrc` or `track.<ext>.lrc`).
* Supports multiple timestamps per line, fractional seconds (1–3 digits), and skips metadata tags (`[ar:]`, `[ti:]`, …).
* The Now panel shows the previous, current (highlighted), and next lyric line, auto-advancing in real time.

### Now Playing Metadata
* Reads **title, artist, and album** tags from your files and displays them in the Now panel, along with the track index.

### YouTube Downloads
* Download audio directly from YouTube from inside the player (`y`), using `yt-dlp`.
* Prefers native **m4a** (no transcode) with embedded metadata, downloaded straight into your library.
* The library refreshes automatically when a download completes. Requires `yt-dlp` on your PATH.

### MPRIS2 (D-Bus) Integration
* Exposes an `org.mpris.MediaPlayer2.trix` D-Bus server, so external controllers just work:
  * `playerctl play-pause`, `playerctl next`, `playerctl previous`, `playerctl stop`
  * Media keys on your keyboard (via your DE's media-key daemon)
  * KDE Connect / phone remote-control apps
* Playback state stays in sync for accurate status queries.

### Terminal Experience
* **Hide-to-shell** — press `F12` to drop out of the TUI into a real PTY-backed subshell (your `$SHELL`); press `F12` again to return. Music keeps playing. The shell auto-respawns if it exits, and window resize is handled properly.
* **Help cheatsheet** — press `h`/`?` for a scrollable, searchable, wrap-aware overlay of every binding.
* **Vim-style navigation** — `j`/`k` and arrow keys.
* **Clean shutdown** — handles `Ctrl+C`, `SIGINT`/`SIGTERM`/`SIGHUP`/`SIGQUIT` and terminal closure gracefully, restoring your terminal on any exit path.
* **Fully themeable** — Atom Dark–inspired default palette with RGB colors; override any element via `~/.config/trix/config.toml`.

## 📦 Installation

### Arch Linux (AUR)
Trix is available on the AUR as [`trix-player`](https://aur.archlinux.org/packages/trix-player). Use your favorite AUR helper:

```bash
yay -S trix-player
# or
paru -S trix-player
```

### Debian / Ubuntu
Download the latest `.deb` file from the [Releases Page](https://github.com/RIZAmohammadkhan/TerminalMusicPlayer/releases).

```bash
sudo dpkg -i trix-player_*.deb
sudo apt-get install -f # Fix dependencies if needed
```

### Fedora / RHEL / openSUSE
Download the latest `.rpm` file from the [Releases Page](https://github.com/RIZAmohammadkhan/TerminalMusicPlayer/releases).

```bash
sudo rpm -i trix-player-*.rpm
```

### Generic Linux (Binary)
Download the `.tar.xz` archive from the Releases page, extract it, and move the binary to your path.

```bash
tar -xf trix-player-*.tar.xz
sudo mv trix /usr/local/bin/
```

### One-line Install (curl)
Installs the latest `x86_64-unknown-linux-gnu` release into `~/.local/bin` (or `/usr/local/bin` if run as root), and installs `yt-dlp` automatically if it's missing.

```bash
curl -fsSL https://raw.githubusercontent.com/RIZAmohammadkhan/TerminalMusicPlayer/main/install.sh | sh
```

Install to a custom prefix:

```bash
curl -fsSL https://raw.githubusercontent.com/RIZAmohammadkhan/TerminalMusicPlayer/main/install.sh | PREFIX=/usr/local sh
```

### From Source (Rust)
If you have the Rust toolchain installed:

```bash
git clone https://github.com/RIZAmohammadkhan/TerminalMusicPlayer.git
cd TerminalMusicPlayer
cargo install --path .
```

## 🚀 Usage

Run the player by typing:

```bash
trix
```

By default, Trix looks for music in your XDG Music directory (usually `~/Music`). You can also play a specific directory or file:

```bash
trix /path/to/my/songs
```

Start playback at a given track index (0-based):

```bash
trix /path/to/my/songs --index 5
```

### The Interface

The TUI is split into a few live panels:

| Panel | Shows |
| :--- | :--- |
| **Title bar** | Playback state, volume (and backend), loop/shuffle/favorites-view indicators, volume-mode hint |
| **Library / Favorites** | Your track list (`▶` marks the current track, `♥` marks favorites, `»` highlights selection) |
| **Search / Move / YouTube box** | The active input mode or a hint to press `S` |
| **Now** | Track, artist, album, and index of the current song |
| **Progress** | A fill bar with the current/total time label |
| **Lyrics** | Previous, current, and next synced lyric line (or a hint if no `.lrc` file exists) |
| **Hints** | Context-sensitive quick tips for the current mode |

## ⌨️ Keyboard Controls

Trix is designed to be used entirely without a mouse.

### Navigation
| Key | Action |
| :--- | :--- |
| `↑` / `k` | Move selection up |
| `↓` / `j` | Move selection down |
| `Enter` | Play selected track |
| `PgUp` / `PgDn` | Scroll page up/down (help overlay) |
| `Home` / `End` | Jump to top/bottom (help overlay) |

### Playback
| Key | Action |
| :--- | :--- |
| `Space` | Pause / Resume |
| `N` | Play next track |
| `P` | Play previous track |
| `l` | Toggle **Loop** current/selected track |
| `s` | Toggle **Shuffle** |
| `r` | Restart current track |

### Seeking
| Key | Action |
| :--- | :--- |
| `→` | Seek forward 5s |
| `←` | Seek backward 5s |
| `n` | Seek forward 10s |
| `p` | Seek backward 10s |

### Favorites
| Key | Action |
| :--- | :--- |
| `a` | Add/Remove selected track from favorites |
| `Tab` | Toggle between Library and Favorites view |

### Modes
| Key | Action |
| :--- | :--- |
| `S` | **Search** mode — type to filter, `Enter` to play, `Esc` to cancel, `Backspace` to delete |
| `m` | **Move** to timestamp — type `1:30` or `01:02:03`, `Enter` to jump, `Esc` to cancel |
| `y` | **YouTube download** — paste a URL, `Enter` to download, `Esc` to cancel |
| `v` | **Volume mode** — `↑`/`↓` (or `k`/`j`) to adjust, `v`/`Esc` to exit |

### Utility
| Key | Action |
| :--- | :--- |
| `F12` | Hide/Unhide Trix to a shell (press again to return, or `exit`; the shell auto-respawns) |
| `D` | **Delete** track from disk (press twice within 2.5s to confirm) |
| `h` / `?` | Toggle **Help** cheatsheet overlay |
| `q` | Quit |
| `Ctrl+C` | Quit (also handles `SIGTERM`/`SIGHUP`) |

## ⚙️ Configuration

Trix loads a user config file from:

- `$XDG_CONFIG_HOME/trix/config.toml`
- or (fallback) `~/.config/trix/config.toml`

This is focused on theming colors.

### Theme colors

Color values can be basic names (e.g. `"cyan"`, `"dark_gray"`) or hex RGB (e.g. `"#61afef"`).

Important semantic constraint:

- `theme.library_accent` is used for both the **Library frame** and the **selection highlight**.
	This is intentional so you can't configure the UI into a confusing state where "library color" and
	"selection color" drift apart.

Example `~/.config/trix/config.toml`:

```toml
[theme]

# Global UI background
# Use "reset" (or omit this key) to respect your terminal background/transparency.
# Set a hex color (e.g. "#282c34") to force a solid UI background.
background = "reset"

title_accent = "#61afef"
current_track_accent = "#56b6c2"
playing_indicator = "#98c379"

# Single knob: library frame + selection highlight
library_accent = "#e5c07b"

now_accent = "#61afef"
progress_accent = "#98c379"
hints_accent = "#c678dd"
search_accent = "#56b6c2"
move_accent = "#e5c07b"

key_accent = "#c678dd"
song_title_accent = "#e5c07b"

text_primary = "#abb2bf"
text_muted = "#5c6370"
error = "#e06c75"
```

The default theme is an **Atom Dark**–inspired palette using standardized RGB colors (with a terminal-respecting transparent background), so colors render consistently across terminals regardless of your ANSI palette. For maximum consistency, prefer hex RGB values in your config.

## 🎧 MPRIS2 Control

Trix registers an MPRIS2 D-Bus service (`org.mpris.MediaPlayer2.trix`) while running, so you can control it from anywhere:

```bash
playerctl play-pause
playerctl next
playerctl previous
playerctl stop
playerctl status   # Playing / Paused / Stopped
```

This also means your desktop's media-key bindings, KDE Connect, or GNOME sound indicator can control Trix without any extra setup.

## 🎵 YouTube Downloads

Press `y`, paste a URL, and press `Enter`. Trix will use `yt-dlp` to download the best available m4a audio directly into your library directory (with metadata embedded), then automatically refresh the library so the new track appears.

Requires [`yt-dlp`](https://github.com/yt-dlp/yt-dlp) to be installed and on your `PATH`. The one-line installer and the AUR package install it for you.

## 🔧 Dependencies

To build or run Trix, you need ALSA development libraries installed on your system.

* **Debian/Ubuntu:** `sudo apt install libasound2-dev`
* **Fedora:** `sudo dnf install alsa-lib-devel`
* **Arch:** `sudo pacman -S alsa-lib`

Optional runtime dependency for the YouTube feature:

* **yt-dlp** — `pipx install yt-dlp` or see the [official install guide](https://github.com/yt-dlp/yt-dlp#installation)

## 🏗️ Building from Source

```bash
git clone https://github.com/RIZAmohammadkhan/TerminalMusicPlayer.git
cd TerminalMusicPlayer
cargo build --release
./target/release/trix
```

## 🤝 Contributing

Contributions are welcome! Feel free to open issues for bugs or feature requests.

1. Fork the repository.
2. Create your feature branch (`git checkout -b feature/AmazingFeature`).
3. Commit your changes (`git commit -m 'Add some AmazingFeature'`).
4. Push to the branch (`git push origin feature/AmazingFeature`).
5. Open a Pull Request.

## Known Bugs
1. Closing the terminal results in a ~1 second lag of audio closure.
2. Named colors may render differently across terminals; prefer hex RGB colors in your config for consistent output.

## 📄 License

Distributed under the MIT License. See `LICENSE` for more information.

## 👤 Author

**Riza Mohammad**
* GitHub: [@RIZAmohammadkhan](https://github.com/RIZAmohammadkhan)

---

<div align="center">
Made with ❤️ and Rust
</div>
