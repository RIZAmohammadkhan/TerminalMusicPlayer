use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result};
use ratatui::style::Color;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub theme: Theme,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };

        match load_from_path(&path) {
            Ok(cfg) => cfg,
            Err(err) => {
                eprintln!("trix: config load failed ({}): {err:#}", path.display());
                Self::default()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    /// Global UI background.
    ///
    /// Default is `Color::Reset` so the UI respects the user's terminal background
    /// (including transparency and custom color schemes).
    pub background: Color,

    pub title_accent: Color,
    pub current_track_accent: Color,
    pub playing_indicator: Color,

    /// Library accent drives both the library frame and the list selection highlight.
    ///
    /// This enforces the semantic constraint that “library color” and “selection color”
    /// stay equal in configuration.
    pub library_accent: Color,

    pub now_accent: Color,
    pub progress_accent: Color,
    pub hints_accent: Color,
    pub search_accent: Color,
    pub move_accent: Color,

    pub key_accent: Color,
    pub song_title_accent: Color,

    pub text_primary: Color,
    pub text_muted: Color,
    pub error: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // Atom Dark / One Dark inspired palette (standardized RGB).
            // Using RGB avoids terminal-specific reinterpretation of ANSI named colors.
            // Background is reset by default to respect terminal theme/transparency.
            // Set `theme.background` in config to force a specific color.
            background: Color::Reset,

            title_accent: Color::Rgb(0x61, 0xaf, 0xef),     // #61afef (blue)
            current_track_accent: Color::Rgb(0x56, 0xb6, 0xc2), // #56b6c2 (cyan)
            playing_indicator: Color::Rgb(0x98, 0xc3, 0x79), // #98c379 (green)

            library_accent: Color::Rgb(0xe5, 0xc0, 0x7b),   // #e5c07b (yellow)

            now_accent: Color::Rgb(0x61, 0xaf, 0xef),       // #61afef (blue)
            progress_accent: Color::Rgb(0x98, 0xc3, 0x79),  // #98c379 (green)
            hints_accent: Color::Rgb(0xc6, 0x78, 0xdd),     // #c678dd (purple)
            search_accent: Color::Rgb(0x56, 0xb6, 0xc2),    // #56b6c2 (cyan)
            move_accent: Color::Rgb(0xe5, 0xc0, 0x7b),      // #e5c07b (yellow)

            key_accent: Color::Rgb(0xc6, 0x78, 0xdd),       // #c678dd (purple)
            song_title_accent: Color::Rgb(0xe5, 0xc0, 0x7b), // #e5c07b (yellow)

            text_primary: Color::Rgb(0xab, 0xb2, 0xbf),     // #abb2bf
            text_muted: Color::Rgb(0x5c, 0x63, 0x70),       // #5c6370
            error: Color::Rgb(0xe0, 0x6c, 0x75),            // #e06c75
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    theme: Option<RawTheme>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTheme {
    background: Option<String>,

    title_accent: Option<String>,
    current_track_accent: Option<String>,
    playing_indicator: Option<String>,

    library_accent: Option<String>,

    now_accent: Option<String>,
    progress_accent: Option<String>,
    hints_accent: Option<String>,
    search_accent: Option<String>,
    move_accent: Option<String>,

    key_accent: Option<String>,
    song_title_accent: Option<String>,

    text_primary: Option<String>,
    text_muted: Option<String>,
    error: Option<String>,
}

fn load_from_path(path: &PathBuf) -> Result<Config> {
    let data = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };

    let raw: RawConfig = toml::from_str(&data).context("parse TOML")?;

    let mut cfg = Config::default();
    if let Some(theme) = raw.theme {
        apply_theme(&mut cfg.theme, theme);
    }

    Ok(cfg)
}

fn apply_theme(out: &mut Theme, raw: RawTheme) {
    apply_color(&mut out.background, raw.background, "theme.background");

    apply_color(&mut out.title_accent, raw.title_accent, "theme.title_accent");
    apply_color(
        &mut out.current_track_accent,
        raw.current_track_accent,
        "theme.current_track_accent",
    );
    apply_color(
        &mut out.playing_indicator,
        raw.playing_indicator,
        "theme.playing_indicator",
    );

    // Semantic constraint: one knob for both library + selection highlight.
    apply_color(
        &mut out.library_accent,
        raw.library_accent,
        "theme.library_accent",
    );

    apply_color(&mut out.now_accent, raw.now_accent, "theme.now_accent");
    apply_color(
        &mut out.progress_accent,
        raw.progress_accent,
        "theme.progress_accent",
    );
    apply_color(
        &mut out.hints_accent,
        raw.hints_accent,
        "theme.hints_accent",
    );
    apply_color(
        &mut out.search_accent,
        raw.search_accent,
        "theme.search_accent",
    );
    apply_color(
        &mut out.move_accent,
        raw.move_accent,
        "theme.move_accent",
    );

    apply_color(&mut out.key_accent, raw.key_accent, "theme.key_accent");
    apply_color(
        &mut out.song_title_accent,
        raw.song_title_accent,
        "theme.song_title_accent",
    );

    apply_color(
        &mut out.text_primary,
        raw.text_primary,
        "theme.text_primary",
    );
    apply_color(&mut out.text_muted, raw.text_muted, "theme.text_muted");
    apply_color(&mut out.error, raw.error, "theme.error");
}

fn apply_color(slot: &mut Color, value: Option<String>, key: &str) {
    let Some(value) = value else { return };
    match parse_color(&value) {
        Some(c) => *slot = c,
        None => {
            eprintln!("trix: ignoring invalid color for {key}: {value}");
        }
    }
}

fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let lower = s.to_ascii_lowercase();

    // Respect terminal defaults
    match lower.as_str() {
        "reset" | "default" | "none" | "transparent" | "terminal" => return Some(Color::Reset),
        _ => {}
    }

    // Hex: #RRGGBB
    if let Some(hex) = lower.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
    }

    // Basic named colors (ratatui palette)
    match lower.as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "dark_gray" | "darkgrey" | "dark_grey" => Some(Color::DarkGray),
        "lightred" | "light_red" => Some(Color::LightRed),
        "lightgreen" | "light_green" => Some(Color::LightGreen),
        "lightyellow" | "light_yellow" => Some(Color::LightYellow),
        "lightblue" | "light_blue" => Some(Color::LightBlue),
        "lightmagenta" | "light_magenta" => Some(Color::LightMagenta),
        "lightcyan" | "light_cyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

fn config_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| {
        env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
    })?;

    Some(base.join("trix").join("config.toml"))
}
