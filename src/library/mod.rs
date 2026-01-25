use std::{env, fs, path::{Path, PathBuf}};

use anyhow::Result;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub(crate) struct Track {
    pub(crate) path: PathBuf,
    pub(crate) display_name: String,
}

pub(crate) fn default_library_path() -> PathBuf {
    default_music_dir().unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub(crate) fn discover_tracks(path: &Path) -> Result<Vec<Track>> {
    let mut files: Vec<PathBuf> = Vec::new();

    if path.is_file() {
        files.push(path.to_path_buf());
    } else {
        for entry in WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let p = entry.path();
            if is_audio_file(p) {
                files.push(p.to_path_buf());
            }
        }
    }

    files.sort();

    let tracks = files
        .into_iter()
        .map(|p| Track {
            display_name: p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string()),
            path: p,
        })
        .collect();

    Ok(tracks)
}

fn is_audio_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus"
    )
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

fn xdg_music_dir() -> Option<PathBuf> {
    let home = home_dir()?;
    let config = home.join(".config/user-dirs.dirs");
    let content = fs::read_to_string(config).ok()?;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some(rest) = line.strip_prefix("XDG_MUSIC_DIR=") else {
            continue;
        };

        let raw = rest.trim();
        let raw = raw.trim_matches('"');
        let expanded = raw.replace("$HOME", &home.to_string_lossy());
        let p = PathBuf::from(expanded);
        if p.is_dir() {
            return Some(p);
        }
    }

    None
}

fn default_music_dir() -> Option<PathBuf> {
    if let Some(p) = xdg_music_dir() {
        return Some(p);
    }

    let home = home_dir()?;
    for name in ["Music", "music"] {
        let p = home.join(name);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}
