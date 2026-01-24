# Terminal Music Player (Rust TUI)

A fast, keyboard-driven terminal music player for Linux terminals (kitty, alacritty, foot, wezterm, etc.).

This project is built for people who hate GUIs and want a clean, responsive, “nvim-like” music experience:

- Rust + `ratatui` TUI (snappy redraws)
- `rodio` audio playback with Symphonia decoding
- Vim-ish, single-key controls + arrow-key seeking
- Track name + time/progress display

## Status

Early but usable: it can scan a folder of audio files, play them, switch tracks, seek, and adjust volume.

## Requirements (Linux)

- Rust toolchain (`cargo`)
- A working audio setup (ALSA / PipeWire / PulseAudio)
- System ALSA development headers (needed by `cpal`/`alsa-sys`):
	- Debian/Ubuntu: `sudo apt install libasound2-dev`
	- Fedora: `sudo dnf install alsa-lib-devel`
	- Arch: `sudo pacman -S alsa-lib`

## Build

```bash
cargo build --release
```

This produces a short executable named `tmp`:

```bash
./target/release/tmp --help
```

## Run

Play a directory (recursively scans for audio files):

```bash
cargo run --release -- /path/to/music
```

Play a single file:

```bash
cargo run --release -- /path/to/song.mp3
```

Start at a specific track index:

```bash
cargo run --release -- --index 10 /path/to/music
```

## Supported formats

File extensions currently detected:

- `mp3`, `flac`, `wav`, `ogg`, `m4a`, `aac`, `opus`

(Actual decode support is provided by Symphonia; the list above is just the library scan filter.)

## Keybindings

Playback:

- `q` quit
- `Space` pause/resume
- `P` previous track
- `N` next track
- `r` restart current track (play from start)
- `l` loop selected/current track
- `s` toggle shuffle order

Seeking:

- `p` seek **-10s**
- `n` seek **+10s**
- `←` seek **-5s**
- `→` seek **+5s**

Volume:

- `v` toggle volume mode
- While volume mode is ON: `↑` volume up, `↓` volume down (`Esc` exits)

Library:

- `↑/↓` select track
- `Enter` play selected
- `S` search (type to select; `Enter` plays; `Esc` cancels)
- `D` delete selected track (press twice to confirm)

## Notes on seeking

Seeking is implemented by restarting decoding at the new offset (simple and reliable). It’s usually fast, but very large files or some formats may seek less smoothly.

## Troubleshooting

- No audio / ALSA error at startup: install the ALSA dev package for your distro (see Requirements) and ensure your system audio works outside this app.
- Terminal looks weird after crash: run `reset` or restart the terminal.

## Roadmap

Planned improvements:

- Better library UX: search/filter, sort by name/mtime, shuffle/repeat
- Metadata: show artist/album/title (tags)
- Real mixer volume (system volume) instead of per-app gain
- Play queue / playlists
