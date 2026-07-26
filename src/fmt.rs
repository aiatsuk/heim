//! Pure formatting helpers — keep UI free of ad-hoc math.

/// Compact dust-style: 538B, 1.3K, 16K, 1.2M
pub fn human_bytes_short(n: u64) -> String {
    const U: [char; 5] = ['B', 'K', 'M', 'G', 'T'];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n}B")
    } else if v >= 10.0 {
        format!("{v:.0}{}", U[i])
    } else {
        format!("{v:.1}{}", U[i])
    }
}

/// Parse dust size tokens: 538B, 1.3K, 16K, 1.2M, 2G
pub fn parse_dust_size(tok: &str) -> Option<u64> {
    let tok = tok.trim();
    if tok.is_empty() {
        return None;
    }
    let (num, mult) = match tok.chars().last()? {
        'B' | 'b' => (&tok[..tok.len() - 1], 1u64),
        'K' | 'k' => (&tok[..tok.len() - 1], 1024),
        'M' | 'm' => (&tok[..tok.len() - 1], 1024 * 1024),
        'G' | 'g' => (&tok[..tok.len() - 1], 1024 * 1024 * 1024),
        'T' | 't' => (&tok[..tok.len() - 1], 1024u64.pow(4)),
        _ => {
            return tok.parse().ok();
        }
    };
    let v: f64 = num.trim().parse().ok()?;
    Some((v * mult as f64).round() as u64)
}

pub fn signed_i64(n: i64) -> String {
    if n > 0 {
        format!("+{n}")
    } else {
        n.to_string()
    }
}

pub fn signed_bytes(n: i64) -> String {
    if n == 0 {
        return "0".into();
    }
    let sign = if n > 0 { "+" } else { "-" };
    format!("{sign}{}", human_bytes_short(n.unsigned_abs()))
}

/// Compact duration for status ages: `45s`, `1m30s`, `2h05m`.
pub fn hum_dur(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Refresh-interval label: `45s`, `1m`, `2m30s`.
pub fn hum_interval(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

pub fn hum_dur_age(d: std::time::Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 1.0 {
        "now".into()
    } else if secs < 60.0 {
        format!("{}s", secs.floor() as u64)
    } else {
        hum_dur(secs.floor() as u64)
    }
}

pub fn num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

pub fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        (part as f64) * 100.0 / (whole as f64)
    }
}

pub fn truncate_middle(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let keep = max - 1;
    let head = keep / 2;
    let tail = keep - head;
    let mut out: String = chars.iter().take(head).collect();
    out.push('…');
    out.extend(chars.iter().skip(chars.len() - tail));
    out
}

pub fn pad_left(s: &str, w: usize) -> String {
    let len = s.chars().count();
    if len >= w {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(w - len))
    }
}

pub fn pad_right(s: &str, w: usize) -> String {
    let len = s.chars().count();
    if len >= w {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(w - len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes() {
        assert_eq!(human_bytes_short(500), "500B");
        assert_eq!(human_bytes_short(2048), "2.0K");
        assert_eq!(human_bytes_short(5 * 1024 * 1024), "5.0M");
    }

    #[test]
    fn parse_dust() {
        assert_eq!(parse_dust_size("538B"), Some(538));
        assert_eq!(parse_dust_size("1.3K"), Some(1331));
        assert_eq!(parse_dust_size("16K"), Some(16384));
        assert_eq!(parse_dust_size("1.2M"), Some(1_258_291));
    }

    #[test]
    fn signed() {
        assert_eq!(signed_i64(12), "+12");
        assert_eq!(signed_i64(-3), "-3");
        assert_eq!(signed_bytes(2048), "+2.0K");
        assert_eq!(signed_bytes(-1024), "-1.0K");
    }

    #[test]
    fn pad_and_truncate() {
        assert_eq!(pad_left("12", 4), "  12");
        assert_eq!(pad_right("12", 4), "12  ");
        assert_eq!(truncate_middle("hello_world", 8).chars().count(), 8);
        assert_eq!(hum_interval(45), "45s");
        assert_eq!(hum_interval(60), "1m");
        assert_eq!(hum_interval(150), "2m30s");
    }
}
