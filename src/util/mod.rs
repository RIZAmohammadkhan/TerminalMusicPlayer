use std::time::{Duration, Instant};

pub(crate) fn make_shuffled_order(len: usize, current: usize) -> Vec<usize> {
    if len == 0 {
        return Vec::new();
    }
    if len == 1 {
        return vec![0];
    }

    let mut rest: Vec<usize> = (0..len).filter(|&i| i != current).collect();

    // Fisher–Yates shuffle.
    for i in (1..rest.len()).rev() {
        let j = fastrand::usize(..=i);
        rest.swap(i, j);
    }

    let mut order = Vec::with_capacity(len);
    order.push(current);
    order.extend(rest);
    order
}

pub(crate) fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    let m = s / 60;
    let s = s % 60;
    format!("{m:02}:{s:02}")
}

pub(crate) fn parse_timestamp(input: &str) -> std::result::Result<Duration, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Enter a timestamp (e.g. 1:30 or 01:02:03).".to_string());
    }

    // Accept: SS, M:SS, HH:MM:SS
    let parts: Vec<&str> = s.split(':').collect();
    let parse_u64 = |p: &str| {
        p.trim()
            .parse::<u64>()
            .map_err(|_| format!("Invalid number: '{p}'"))
    };

    let (h, m, sec) = match parts.len() {
        1 => {
            let sec = parse_u64(parts[0])?;
            (0u64, 0u64, sec)
        }
        2 => {
            let m = parse_u64(parts[0])?;
            let sec = parse_u64(parts[1])?;
            (0u64, m, sec)
        }
        3 => {
            let h = parse_u64(parts[0])?;
            let m = parse_u64(parts[1])?;
            let sec = parse_u64(parts[2])?;
            (h, m, sec)
        }
        _ => {
            return Err(
                "Invalid timestamp format. Use SS, M:SS, or HH:MM:SS (e.g. 1:30, 01:02:03)."
                    .to_string(),
            );
        }
    };

    if parts.len() >= 2 && sec >= 60 {
        return Err("Seconds must be 0..59 (use M:SS / HH:MM:SS).".to_string());
    }
    if parts.len() == 3 && m >= 60 {
        return Err("Minutes must be 0..59 in HH:MM:SS.".to_string());
    }

    let total_secs = h
        .saturating_mul(3600)
        .saturating_add(m.saturating_mul(60))
        .saturating_add(sec);

    Ok(Duration::from_secs(total_secs))
}

pub(crate) trait SaturatingDurationSince {
    fn saturating_duration_since(self, earlier: Instant) -> Duration;
}

impl SaturatingDurationSince for Instant {
    fn saturating_duration_since(self, earlier: Instant) -> Duration {
        if self >= earlier {
            self.duration_since(earlier)
        } else {
            Duration::ZERO
        }
    }
}
