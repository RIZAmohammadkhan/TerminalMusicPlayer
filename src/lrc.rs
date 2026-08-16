use std::fs;
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug)]
pub(crate) struct LrcEntry {
    pub(crate) time: Duration,
    pub(crate) text: String,
}

/// Loads and parses the `.lrc` file corresponding to `audio_path` if it exists.
///
/// Tries both `track.lrc` (standard) and `track.<ext>.lrc` (some apps).
/// Returns `None` if no file is found or parsing yields no entries.
pub(crate) fn load_lrc(audio_path: &Path) -> Option<Vec<LrcEntry>> {
    let mut p = audio_path.to_path_buf();
    p.set_extension("lrc");
    if let Ok(content) = fs::read_to_string(&p) {
        let entries = parse_lrc(&content);
        if !entries.is_empty() {
            return Some(entries);
        }
    }

    // Fallback: try `track.<ext>.lrc`.
    let p2 = {
        let mut q = audio_path.to_path_buf();
        if let Some(ext) = q.extension() {
            let new_ext = format!("{}.lrc", ext.to_string_lossy());
            q.set_extension(new_ext);
        } else {
            q.set_extension("lrc");
        }
        q
    };
    if let Ok(content) = fs::read_to_string(&p2) {
        let entries = parse_lrc(&content);
        if !entries.is_empty() {
            return Some(entries);
        }
    }
    None
}

/// Parses LRC-formatted text into a list of timed lyric entries.
///
/// Supports:
/// - Multiple timestamps per line: `[00:01.00][00:05.00]Lyrics text`
/// - Fractional seconds with 1-3 digits: `[mm:ss.f]`, `[mm:ss.ff]`, `[mm:ss.fff]`
/// - Skips metadata-only lines such as `[ar:Artist]`, `[ti:Title]`.
pub(crate) fn parse_lrc(content: &str) -> Vec<LrcEntry> {
    let mut entries: Vec<LrcEntry> = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim_end();
        if line.is_empty() {
            continue;
        }

        let mut rest = line;
        let mut times: Vec<Duration> = Vec::new();

        while rest.starts_with('[') {
            let Some(close) = rest.find(']') else { break };
            let tag = &rest[1..close];
            match parse_lrc_time(tag) {
                Some(t) => times.push(t),
                None => {
                    // Metadata tag like [ar:Artist], [ti:Title]. Skip the whole line.
                    times.clear();
                    break;
                }
            }
            rest = &rest[close + 1..];
        }

        if times.is_empty() {
            continue;
        }

        let text = rest.trim().to_string();
        for t in times {
            entries.push(LrcEntry {
                time: t,
                text: text.clone(),
            });
        }
    }

    entries.sort_by_key(|e| e.time);
    entries
}

fn parse_lrc_time(s: &str) -> Option<Duration> {
    // Accepts: mm:ss, mm:ss.f, mm:ss.ff, mm:ss.fff, m:ss
    let parts: Vec<&str> = s.splitn(2, ':').collect();
    if parts.len() == 1 {
        return None;
    }

    let mm: u64 = parts[0].parse().ok()?;
    let rest = parts[1];

    let (ss_str, frac_str) = match rest.find('.') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };

    let ss: u64 = ss_str.parse().ok()?;
    let frac_ms: u64 = match frac_str {
        None => 0,
        Some(f) => {
            let f = f.trim();
            if f.is_empty() || !f.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let len = f.len().min(3);
            let f = &f[..len];
            let n: u64 = f.parse().ok()?;
            let mult = 10u64.pow(3 - len as u32);
            n * mult
        }
    };

    Some(Duration::from_millis(mm * 60 * 1000 + ss * 1000 + frac_ms))
}

/// Returns the index of the last entry whose time is `<= pos`.
/// Returns `None` if `pos` is before the first entry (or the list is empty).
pub(crate) fn active_index(entries: &[LrcEntry], pos: Duration) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    let pos_ms = pos.as_millis() as u64;

    if entries
        .first()
        .map(|e| e.time.as_millis() as u64)
        .unwrap_or(u64::MAX)
        > pos_ms
    {
        return None;
    }

    let mut lo: usize = 0;
    let mut hi: usize = entries.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if entries[mid].time.as_millis() as u64 <= pos_ms {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        None
    } else {
        Some(lo - 1)
    }
}