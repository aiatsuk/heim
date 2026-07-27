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
    // Roll over when *rounding* would reach the next unit. Deciding the unit
    // before rounding printed 1_048_575 bytes as "1024K" instead of "1.0M",
    // since 1023.999 fails the `>= 1024.0` test but formats as "1024".
    if v >= 1023.95 && i < U.len() - 1 {
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
    } else if secs.is_multiple_of(60) {
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
        if i > 0 && i.is_multiple_of(3) {
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

/// CPU usage with one decimal: `42.6%`.
pub fn fmt_cpu(pct: f32) -> String {
    let p = pct.clamp(0.0, 100.0);
    format!("{p:.1}%")
}

/// Used/total RAM pair: `3.11G/4G` (same unit scale as [`human_bytes_short`]).
pub fn fmt_mem_pair(used: u64, total: u64) -> String {
    format!("{}/{}", human_bytes_short(used), human_bytes_short(total))
}

/// Network rate from bits/s → `68.442 Mbps` (decimal megabits).
pub fn fmt_mbps(bits_per_sec: f64) -> String {
    let bps = bits_per_sec.max(0.0);
    let mbps = bps / 1_000_000.0;
    if mbps >= 100.0 {
        format!("{mbps:.1} Mbps")
    } else if mbps >= 0.01 {
        format!("{mbps:.3} Mbps")
    } else if bps >= 1000.0 {
        // Sub-Mbps but non-trivial: show Kbps.
        format!("{:.1} Kbps", bps / 1000.0)
    } else {
        format!("{bps:.0} bps")
    }
}

/// ICMP RTT: `14.2 ms`.
pub fn fmt_ping_ms(ms: f64) -> String {
    let ms = ms.max(0.0);
    if ms >= 100.0 {
        format!("{ms:.0} ms")
    } else if ms >= 10.0 {
        format!("{ms:.1} ms")
    } else {
        format!("{ms:.2} ms")
    }
}

/// Compact RTT number without unit (for avg/max pairs): `6.2`, `12`, `150`.
pub fn fmt_ping_short(ms: f64) -> String {
    let ms = ms.max(0.0);
    if ms >= 100.0 {
        format!("{ms:.0}")
    } else if ms >= 10.0 {
        format!("{ms:.1}")
    } else {
        format!("{ms:.2}")
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
        // Unit boundaries: these used to render as "1024B", "1024K", "1024M".
        assert_eq!(human_bytes_short(1023), "1023B");
        assert_eq!(human_bytes_short(1024), "1.0K");
        assert_eq!(human_bytes_short(1_048_575), "1.0M");
        assert_eq!(human_bytes_short(1_048_576), "1.0M");
        assert_eq!(human_bytes_short(1_073_741_823), "1.0G");
        // The unit table tops out at T; no rollover past it, and no panic.
        assert_eq!(human_bytes_short(u64::MAX), "16777216T");
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

    #[test]
    fn host_formatters() {
        assert_eq!(fmt_cpu(42.64), "42.6%");
        assert_eq!(fmt_cpu(0.0), "0.0%");
        assert_eq!(fmt_cpu(100.0), "100.0%");
        assert_eq!(fmt_cpu(150.0), "100.0%");

        // ~3.11 GiB / 4 GiB
        let used = (3.11 * 1024.0 * 1024.0 * 1024.0) as u64;
        let total = 4u64 * 1024 * 1024 * 1024;
        let pair = fmt_mem_pair(used, total);
        assert!(pair.contains('/'), "{pair}");
        assert!(pair.ends_with('G') || pair.contains("G/"), "{pair}");

        assert_eq!(fmt_mbps(68.442e6), "68.442 Mbps");
        assert_eq!(fmt_mbps(75.497e6), "75.497 Mbps");
        assert_eq!(fmt_mbps(0.0), "0 bps");
        assert_eq!(fmt_mbps(500_000.0), "0.500 Mbps");
        assert!(fmt_mbps(8_000.0).contains("Kbps"), "{}", fmt_mbps(8_000.0));

        assert_eq!(fmt_ping_ms(14.2), "14.2 ms");
        assert_eq!(fmt_ping_ms(0.43), "0.43 ms");
        assert_eq!(fmt_ping_ms(150.0), "150 ms");
        assert_eq!(fmt_ping_short(14.2), "14.2");
        assert_eq!(fmt_ping_short(6.25), "6.25");
        assert_eq!(fmt_ping_short(150.0), "150");
    }
}
