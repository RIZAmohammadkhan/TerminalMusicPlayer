# Trix Player

<div align="center">

![Trix Version](https://img.shields.io/github/v/release/RIZAmohammadkhan/TerminalMusicPlayer?label=version)
![AUR Version](https://img.shields.io/aur/version/trix-player)
![License](https://img.shields.io/github/license/RIZAmohammadkhan/TerminalMusicPlayer)
![Build Status](https://img.shields.io/github/actions/workflow/status/RIZAmohammadkhan/TerminalMusicPlayer/release.yml)

**A beautiful, keyboard-driven terminal music player for Linux.**

[Features](#features) • [Installation](#installation) • [Usage](#usage) • [Building](#building-from-source)

</div>

---

**Trix** is a lightweight, high-performance music player built with Rust. It is designed for Linux users who prefer the terminal, offering a clean TUI (Terminal User Interface), robust keyboard navigation (vim-like), and low resource usage.

[![Watch the demo](assets/image.png)](assets/demo.mp4)

## 🎬 Demo
Direct link: [assets/demo.mp4](assets/demo.mp4)

## ✨ Features

*   **Format Support:** MP3, FLAC, WAV, OGG, M4A, AAC, Opus.
*   **Intuitive TUI:** Clean interface built with `ratatui`.
*   **Volume Control:** Native ALSA system volume control (with software fallback).
*   **Search:** Fast, fuzzy-like filtering to find tracks instantly.
*   **Playback Control:** Shuffle, Loop (track/playlist), and Seek.
*   **File Management:** Delete tracks directly from the player.
*   **Standard Compliance:** automatically detects music via `XDG_MUSIC_DIR` or defaults to `~/Music`.

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
Installs the latest `x86_64-unknown-linux-gnu` release into `~/.local/bin` (or `/usr/local/bin` if run as root).

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

## ⚙️ Configuration

Trix can load a user config file from:

- `$XDG_CONFIG_HOME/trix/config.toml`
- or (fallback) `~/.config/trix/config.toml`

Currently this is focused on theming colors.

### Theme colors

Color values can be basic names (e.g. `"cyan"`, `"dark_gray"`) or hex RGB (e.g. `"#7aa2f7"`).

Important semantic constraint:

- `theme.library_accent` is used for both the **Library** frame and the **selection highlight**.
	This is intentional so you can't configure the UI into a confusing state where “library color” and
	“selection color” drift apart.

Example `~/.config/trix/config.toml`:

```toml
[theme]
title_accent = "#7aa2f7"
current_track_accent = "#7aa2f7"
playing_indicator = "#9ece6a"

# Single knob: library frame + selection highlight
library_accent = "#e0af68"

now_accent = "#7aa2f7"
progress_accent = "#9ece6a"
hints_accent = "#7dcfff"
search_accent = "#bb9af7"
move_accent = "#e0af68"

key_accent = "#bb9af7"
song_title_accent = "#e0af68"

text_primary = "white"
text_muted = "dark_gray"
error = "red"
```

### Keyboard Controls

Trix is designed to be used entirely without a mouse.

| Key | Action |
| :--- | :--- |
| **Navigation** | |
| `↑`  | Move selection up |
| `↓`  | Move selection down |
| `Enter` | Play selected track |
| `PgUp` / `PgDn` | Scroll page up/down |
| **Playback** | |
| `Space` | Pause / Resume |
| `N` | Play next track |
| `P` | Play previous track |
| `l` | Toggle **Loop** (Current track) |
| `s` | Toggle **Shuffle** |
| `r` | Restart current track |
| **Seeking** | |
| `→` | Seek forward 5s (per press) |
| `←` | Seek backward 5s (per press) |
| `n` | Seek forward 10s |
| `p` | Seek backward 10s |
| **Utility** | |
| `S` | **Search** mode (Type to filter, `Enter` to play) |
| `m` | **Move** to timestamp (e.g. `1:30`, `01:02:03`) |
| `v` | **Volume** mode (Use `↑`/`↓` to adjust, `v`/`Esc` to exit) |
| `F12` | Hide/Unhide Trix (shell; press again to return, or `exit`) |
| `D` | **Delete** track (Press twice to confirm) |
| `h` / `?` | Toggle **Help** / Cheatsheet |
| `q` | Quit |

## 🔧 Dependencies

To build or run Trix, you need ALSA development libraries installed on your system.

*   **Debian/Ubuntu:** `sudo apt install libasound2-dev`
*   **Fedora:** `sudo dnf install alsa-lib-devel`
*   **Arch:** `sudo pacman -S alsa-lib`

## 🤝 Contributing

Contributions are welcome! Feel free to open issues for bugs or feature requests.

1.  Fork the repository.
2.  Create your feature branch (`git checkout -b feature/AmazingFeature`).
3.  Commit your changes (`git commit -m 'Add some AmazingFeature'`).
4.  Push to the branch (`git push origin feature/AmazingFeature`).
5.  Open a Pull Request.

## Known Bugs
1. closing terminal results in 1 second lag of audio closure
2. colors don't work properly need more standardization instead of terminal specific

## 📄 License

Distributed under the MIT License. See `LICENSE` for more information.

## 👤 Author

**Riza Mohammad**
*   GitHub: [@RIZAmohammadkhan](https://github.com/RIZAmohammadkhan)

---

<div align="center">
Made with ❤️ and Rust
</div>