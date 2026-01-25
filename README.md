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

![Trix Player Screenshot](assets/image.png)

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

### Keyboard Controls

Trix is designed to be used entirely without a mouse.

| Key | Action |
| :--- | :--- |
| **Navigation** | |
| `↑` / `k` | Move selection up |
| `↓` / `j` | Move selection down |
| `Enter` | Play selected track |
| `PgUp` / `PgDn` | Scroll page up/down |
| `Home` / `End` | Jump to top/bottom |
| **Playback** | |
| `Space` | Pause / Resume |
| `N` | Play next track |
| `P` | Play previous track |
| `l` | Toggle **Loop** (Current track) |
| `s` | Toggle **Shuffle** |
| `r` | Restart current track |
| **Seeking** | |
| `→` | Seek forward 5s |
| `←` | Seek backward 5s |
| `n` | Seek forward 10s |
| `p` | Seek backward 10s |
| **Utility** | |
| `S` | **Search** mode (Type to filter, `Enter` to play) |
| `v` | **Volume** mode (Use `↑`/`↓` to adjust, `v`/`Esc` to exit) |
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


## 📄 License

Distributed under the MIT License. See `LICENSE` for more information.

## 👤 Author

**Riza Mohammad**
*   GitHub: [@RIZAmohammadkhan](https://github.com/RIZAmohammadkhan)

---

<div align="center">
Made with ❤️ and Rust
</div>