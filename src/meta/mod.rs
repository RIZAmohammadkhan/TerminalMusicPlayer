use std::{
    fs::File,
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result};
use symphonia::core::{
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey},
    probe::Hint,
    units::Time,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct TrackMeta {
    pub(crate) title: Option<String>,
    pub(crate) artist: Option<String>,
    pub(crate) album: Option<String>,
    pub(crate) duration: Option<Duration>,
}

pub(crate) fn probe_duration(path: &Path) -> Result<Duration> {
    let meta = probe_track_meta(path)?;
    meta.duration.context("Duration unavailable")
}

pub(crate) fn probe_track_meta(path: &Path) -> Result<TrackMeta> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut meta = TrackMeta::default();

    // Gather tags from both the probe metadata and container metadata.
    if let Some(container_meta) = probed.metadata.get() {
        if let Some(rev) = container_meta.current() {
            apply_tags(&mut meta, rev);
        }
    }
    if let Some(rev) = probed.format.metadata().current() {
        apply_tags(&mut meta, rev);
    }

    // Duration (best-effort): use time_base*n_frames if present; else sample_rate*n_frames.
    if meta.duration.is_none() {
        if let Some(track) = probed
            .format
            .default_track()
            .or_else(|| probed.format.tracks().first())
        {
            let params = &track.codec_params;

            if let (Some(time_base), Some(n_frames)) = (params.time_base, params.n_frames) {
                let Time { seconds, frac, .. } = time_base.calc_time(n_frames);
                meta.duration = Some(Duration::from_secs(seconds) + Duration::from_secs_f64(frac));
            } else if let (Some(sample_rate), Some(n_frames)) = (params.sample_rate, params.n_frames)
            {
                let secs = n_frames as f64 / sample_rate as f64;
                if secs.is_finite() && secs > 0.0 {
                    meta.duration = Some(Duration::from_secs_f64(secs));
                }
            }
        }
    }

    Ok(meta)
}

fn apply_tags(meta: &mut TrackMeta, rev: &symphonia::core::meta::MetadataRevision) {
    for tag in rev.tags() {
        let value = tag.value.to_string();
        match tag.std_key {
            Some(StandardTagKey::TrackTitle) => {
                meta.title.get_or_insert(value);
            }
            Some(StandardTagKey::Artist) => {
                meta.artist.get_or_insert(value);
            }
            Some(StandardTagKey::Album) => {
                meta.album.get_or_insert(value);
            }
            _ => {
                // Fallbacks for common raw keys.
                match tag.key.to_ascii_lowercase().as_str() {
                    "title" => {
                        meta.title.get_or_insert(value);
                    }
                    "artist" => {
                        meta.artist.get_or_insert(value);
                    }
                    "album" => {
                        meta.album.get_or_insert(value);
                    }
                    _ => {
                        // ignore
                    }
                }
            }
        };
    }
}
