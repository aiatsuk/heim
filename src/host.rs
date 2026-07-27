//! Live host resource sampling (CPU %, RAM, network rates, ping).
//!
//! Independent of project [`crate::collect::collect`]. Each metric group has
//! its own cadence (see interval constants); the TUI can force network + ping
//! with a key without waiting for the long net period.
//!
//! Successful external pings are kept in an in-memory ring for sparkline +
//! avg/max windows (5m / 10m / 30m). Not persisted to disk.

use std::collections::VecDeque;
use std::process::Command;
use std::time::{Duration, Instant};

use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};

/// CPU + RAM poll interval.
pub const CPU_MEM_INTERVAL: Duration = Duration::from_secs(3);
/// Network rate sample interval (average since previous sample).
/// Cheap: only interface counters via sysinfo — no speedtest traffic.
pub const NET_INTERVAL: Duration = Duration::from_secs(3);
/// ICMP ping interval.
pub const PING_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How long to retain ping samples for sparkline / window stats.
pub const PING_HISTORY_RETAIN: Duration = Duration::from_secs(45 * 60);

/// Windows for avg/max (aligned with project Δ labels).
pub const PING_WINDOWS: &[(u64, &str)] = &[(5 * 60, "5m"), (10 * 60, "10m"), (30 * 60, "30m")];

/// Sparkline covers this trailing window.
pub const PING_SPARK_WINDOW: Duration = Duration::from_secs(30 * 60);

/// External anycast DNS targets from *different* operators (avg for internet RTT).
/// IPs only — never hostnames, so DNS outages do not skew the measurement.
pub const PING_EXTERNAL: &[&str] = &["1.1.1.1", "8.8.8.8"];

const PING_TIMEOUT: Duration = Duration::from_secs(3);

/// Unicode block elements for RTT sparkline (low → high).
const SPARK_BARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Result of one multi-target ping pass (background worker → UI).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PingResult {
    /// Mean RTT over successful external probes (`1.1.1.1`, `8.8.8.8`).
    pub external_ms: Option<f64>,
    /// RTT to the default gateway (local LAN / Wi‑Fi path).
    pub gateway_ms: Option<f64>,
}

/// One recorded external ping (for history / sparkline).
#[derive(Debug, Clone, Copy)]
pub struct PingSample {
    pub at: Instant,
    pub external_ms: f64,
}

/// Avg / max / min over a wall-clock window of retained samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PingWindowStats {
    pub avg: f64,
    pub max: f64,
    pub min: f64,
    pub n: usize,
}

/// One host snapshot for display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostStats {
    /// Aggregate CPU usage, 0..100.
    pub cpu_pct: f32,
    /// Bytes of RAM in use (sysinfo: total − available when available).
    pub mem_used: u64,
    pub mem_total: u64,
    /// Download rate in bits/s once a prior network sample exists.
    pub down_bps: Option<f64>,
    /// Upload rate in bits/s once a prior network sample exists.
    pub up_bps: Option<f64>,
    /// Mean external DNS anycast RTT (ms).
    pub ping_ms: Option<f64>,
    /// Default-gateway RTT (ms) — local path.
    pub ping_gw_ms: Option<f64>,
}

impl Default for HostStats {
    fn default() -> Self {
        Self {
            cpu_pct: 0.0,
            mem_used: 0,
            mem_total: 0,
            down_bps: None,
            up_bps: None,
            ping_ms: None,
            ping_gw_ms: None,
        }
    }
}

/// Stateful sampler: holds sysinfo handles and previous network counters.
pub struct HostMonitor {
    sys: System,
    nets: Networks,
    prev_rx: Option<u64>,
    prev_tx: Option<u64>,
    prev_net_at: Option<Instant>,
    /// True after the first CPU refresh so `global_cpu_usage` is meaningful.
    cpu_primed: bool,
    /// Latest merged snapshot (CPU/mem/net/ping update independently).
    latest: HostStats,
    /// In-memory ping history (successful external RTTs only).
    ping_history: VecDeque<PingSample>,
}

impl HostMonitor {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        let nets = Networks::new_with_refreshed_list();
        Self {
            sys,
            nets,
            prev_rx: None,
            prev_tx: None,
            prev_net_at: None,
            cpu_primed: false,
            latest: HostStats::default(),
            ping_history: VecDeque::new(),
        }
    }

    /// Refresh CPU + memory only. Network and ping fields are left as-is.
    pub fn poll_cpu_mem(&mut self) -> HostStats {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();

        let cpu_pct = if self.cpu_primed {
            self.sys.global_cpu_usage().clamp(0.0, 100.0)
        } else {
            self.cpu_primed = true;
            0.0
        };

        let mem_total = self.sys.total_memory();
        let mem_used = self.sys.used_memory().min(mem_total);

        self.latest.cpu_pct = cpu_pct;
        self.latest.mem_used = mem_used;
        self.latest.mem_total = mem_total;
        self.latest
    }

    /// Refresh network counters and recompute rates since the previous net sample.
    ///
    /// Rates stay `None` until a prior sample exists (and Δt > 0).
    pub fn poll_network(&mut self) -> HostStats {
        self.nets.refresh(true);
        let (rx, tx) = sum_net_bytes(&self.nets);
        let now = Instant::now();
        let (down_bps, up_bps) = match (self.prev_rx, self.prev_tx, self.prev_net_at) {
            (Some(prx), Some(ptx), Some(pat)) => {
                let dt = now.saturating_duration_since(pat).as_secs_f64();
                if dt > 0.0 {
                    let d_rx = rx.saturating_sub(prx) as f64;
                    let d_tx = tx.saturating_sub(ptx) as f64;
                    // bytes/s → bits/s
                    (Some(d_rx * 8.0 / dt), Some(d_tx * 8.0 / dt))
                } else {
                    (self.latest.down_bps, self.latest.up_bps)
                }
            }
            _ => (None, None),
        };
        self.prev_rx = Some(rx);
        self.prev_tx = Some(tx);
        self.prev_net_at = Some(now);
        self.latest.down_bps = down_bps;
        self.latest.up_bps = up_bps;
        self.latest
    }

    /// Apply a completed multi-target ping (from a background worker).
    pub fn apply_ping(&mut self, r: PingResult) -> HostStats {
        self.latest.ping_ms = r.external_ms;
        self.latest.ping_gw_ms = r.gateway_ms;
        if let Some(ms) = r.external_ms {
            if ms.is_finite() && ms >= 0.0 {
                self.ping_history.push_back(PingSample {
                    at: Instant::now(),
                    external_ms: ms,
                });
                self.trim_ping_history();
            }
        }
        self.latest
    }

    fn trim_ping_history(&mut self) {
        let cutoff = Instant::now()
            .checked_sub(PING_HISTORY_RETAIN)
            .unwrap_or_else(Instant::now);
        while self.ping_history.front().is_some_and(|s| s.at < cutoff) {
            self.ping_history.pop_front();
        }
    }

    /// Samples with `at` still inside `window` ending now.
    pub fn ping_samples_in(&self, window: Duration) -> Vec<f64> {
        let cutoff = Instant::now()
            .checked_sub(window)
            .unwrap_or_else(Instant::now);
        self.ping_history
            .iter()
            .filter(|s| s.at >= cutoff)
            .map(|s| s.external_ms)
            .collect()
    }

    pub fn ping_window_stats(&self, window: Duration) -> Option<PingWindowStats> {
        let vals = self.ping_samples_in(window);
        if vals.is_empty() {
            return None;
        }
        let n = vals.len();
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut sum = 0.0;
        for v in &vals {
            min = min.min(*v);
            max = max.max(*v);
            sum += *v;
        }
        Some(PingWindowStats {
            avg: sum / n as f64,
            max,
            min,
            n,
        })
    }

    /// Sparkline of external RTT over [`PING_SPARK_WINDOW`] (oldest → newest).
    pub fn ping_sparkline(&self, width: usize) -> String {
        let vals = self.ping_samples_in(PING_SPARK_WINDOW);
        sparkline(&vals, width)
    }

    /// How many external ping samples are retained.
    pub fn ping_history_len(&self) -> usize {
        self.ping_history.len()
    }

    /// Full one-shot sample for `--once`: CPU/mem (primed), short net window, pings.
    ///
    /// Returns stats and the monitor (history has the one-shot ping sample).
    pub fn sample_once() -> (HostStats, Self) {
        let mut m = Self::new();
        let _ = m.poll_cpu_mem();
        std::thread::sleep(Duration::from_millis(200));
        let _ = m.poll_cpu_mem();
        let _ = m.poll_network();
        std::thread::sleep(Duration::from_millis(400));
        let _ = m.poll_network();
        let stats = m.apply_ping(measure_all_pings());
        (stats, m)
    }
}

/// Build a block-element sparkline. Empty → `—`; single value → mid bar.
pub fn sparkline(values: &[f64], width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if values.is_empty() {
        return "—".repeat(width.min(1));
    }
    let sampled: Vec<f64> = if values.len() <= width {
        values.to_vec()
    } else {
        // Evenly spaced pick across history.
        (0..width)
            .map(|i| {
                let idx = i * (values.len() - 1) / (width - 1).max(1);
                values[idx]
            })
            .collect()
    };
    let min = sampled.iter().copied().fold(f64::INFINITY, f64::min);
    let max = sampled.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1e-9);
    let levels = (SPARK_BARS.len() - 1) as f64;
    sampled
        .iter()
        .map(|v| {
            let t = ((v - min) / span).clamp(0.0, 1.0);
            let i = (t * levels).round() as usize;
            SPARK_BARS[i.min(SPARK_BARS.len() - 1)]
        })
        .collect()
}

/// Compact history line: sparkline + avg/max windows.
///
/// Example: `ping ▁▂▃▅█  5m 6.2/12  10m 7.1/18  30m 8.0/40` (avg/max ms)
pub fn format_ping_history_line(mon: &HostMonitor, width: usize) -> String {
    let spark_w = if width >= 48 {
        12
    } else if width >= 36 {
        8
    } else {
        6
    };
    let spark = mon.ping_sparkline(spark_w);
    let mut parts = vec![format!("ping {spark}")];
    for &(secs, label) in PING_WINDOWS {
        match mon.ping_window_stats(Duration::from_secs(secs)) {
            Some(st) => parts.push(format!(
                "{label} {}/{}",
                crate::fmt::fmt_ping_short(st.avg),
                crate::fmt::fmt_ping_short(st.max)
            )),
            None => parts.push(format!("{label} —")),
        }
    }
    let line = parts.join("  ");
    if line.chars().count() <= width {
        line
    } else {
        crate::fmt::truncate_middle(&line, width)
    }
}

impl Default for HostMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Probe Cloudflare + Google (average) and the default gateway.
///
/// Call off the UI thread — up to a few seconds of sequential ICMP.
pub fn measure_all_pings() -> PingResult {
    let mut external: Vec<f64> = Vec::with_capacity(PING_EXTERNAL.len());
    for host in PING_EXTERNAL {
        if let Some(ms) = measure_ping(host) {
            external.push(ms);
        }
    }
    let external_ms = if external.is_empty() {
        None
    } else {
        Some(external.iter().sum::<f64>() / external.len() as f64)
    };
    let gateway_ms = default_gateway().and_then(|gw| measure_ping(&gw));
    PingResult {
        external_ms,
        gateway_ms,
    }
}

/// Blocking ICMP ping via the system `ping` binary. Call off the UI thread.
///
/// Returns RTT in milliseconds, or `None` on timeout / parse / spawn failure.
pub fn measure_ping(host: &str) -> Option<f64> {
    let mut cmd = Command::new("ping");
    #[cfg(target_os = "linux")]
    {
        // -c 1: one probe; -W 2: wait up to 2s for a reply (seconds on Linux).
        cmd.args(["-c", "1", "-W", "2", host]);
    }
    #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd"))]
    {
        // -W is milliseconds on BSD/macOS.
        cmd.args(["-c", "1", "-W", "2000", host]);
    }
    #[cfg(windows)]
    {
        cmd.args(["-n", "1", "-w", "2000", host]);
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        windows
    )))]
    {
        cmd.args(["-c", "1", host]);
    }

    let out = crate::collect::run_timed(cmd, PING_TIMEOUT).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_ping_ms(&text).or_else(|| {
        let err = String::from_utf8_lossy(&out.stderr);
        parse_ping_ms(&err)
    })
}

/// Default IPv4 gateway (router). Prefer pure `/proc` on Linux; fall back to
/// `ip` / `route` helpers elsewhere.
pub fn default_gateway() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Some(gw) = gateway_from_proc_route() {
            return Some(gw);
        }
    }
    gateway_from_ip_route().or_else(gateway_from_route_get)
}

/// Parse Linux `/proc/net/route` for the default route's gateway.
#[cfg(target_os = "linux")]
fn gateway_from_proc_route() -> Option<String> {
    let text = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in text.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let _iface = cols.next()?;
        let dest = cols.next()?;
        let gateway = cols.next()?;
        // Destination 00000000 = default route.
        if dest != "00000000" {
            continue;
        }
        // Gateway is little-endian hex, e.g. 0101A8C0 → 192.168.1.1
        let n = u32::from_str_radix(gateway, 16).ok()?;
        if n == 0 {
            continue;
        }
        let b = n.to_le_bytes();
        return Some(format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]));
    }
    None
}

/// `ip route show default` → `default via 192.168.1.1 dev eth0 …`
fn gateway_from_ip_route() -> Option<String> {
    let mut cmd = Command::new("ip");
    cmd.args(["route", "show", "default"]);
    let out = crate::collect::run_timed(cmd, Duration::from_secs(2)).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_via_gateway(&text)
}

/// macOS/BSD: `route -n get default` → `gateway: 192.168.1.1`
fn gateway_from_route_get() -> Option<String> {
    let mut cmd = Command::new("route");
    cmd.args(["-n", "get", "default"]);
    let out = crate::collect::run_timed(cmd, Duration::from_secs(2)).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("gateway:") {
            let gw = rest.trim();
            if !gw.is_empty() && gw != "0.0.0.0" {
                return Some(gw.to_string());
            }
        }
    }
    None
}

fn parse_via_gateway(text: &str) -> Option<String> {
    // default via 192.168.1.1 dev eth0
    let mut words = text.split_whitespace();
    while let Some(w) = words.next() {
        if w == "via" {
            let gw = words.next()?;
            if gw != "0.0.0.0" && !gw.is_empty() {
                return Some(gw.to_string());
            }
        }
    }
    None
}

/// Extract RTT ms from common `ping` output forms (`time=12.3 ms`, `time=12ms`).
pub fn parse_ping_ms(text: &str) -> Option<f64> {
    for token in text.split_whitespace() {
        // time=12.3ms  or  time=12.3  or  time<1ms
        let Some(rest) = token
            .strip_prefix("time=")
            .or_else(|| token.strip_prefix("time<"))
        else {
            continue;
        };
        let num = rest.trim_end_matches("ms").trim_end_matches("MS");
        if let Ok(v) = num.parse::<f64>() {
            if v.is_finite() && v >= 0.0 {
                return Some(v);
            }
        }
    }
    // Fallback: "time=12.3 ms" where ms is a separate token after =
    if let Some(i) = text.find("time=") {
        let tail = &text[i + 5..];
        let num: String = tail
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if let Ok(v) = num.parse::<f64>() {
            if v.is_finite() && v >= 0.0 {
                return Some(v);
            }
        }
    }
    None
}

fn sum_net_bytes(nets: &Networks) -> (u64, u64) {
    let mut rx = 0u64;
    let mut tx = 0u64;
    for (name, data) in nets {
        if is_loopback(name) {
            continue;
        }
        rx = rx.saturating_add(data.total_received());
        tx = tx.saturating_add(data.total_transmitted());
    }
    (rx, tx)
}

fn is_loopback(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "lo" || n == "lo0" || n.starts_with("lo:") || n.starts_with("loopback") || n == "localhost"
}

fn fmt_ping_pair(s: &HostStats) -> (String, String) {
    let ping = s
        .ping_ms
        .map(crate::fmt::fmt_ping_ms)
        .unwrap_or_else(|| "—".into());
    let gw = s
        .ping_gw_ms
        .map(crate::fmt::fmt_ping_ms)
        .unwrap_or_else(|| "—".into());
    (ping, gw)
}

/// Compact single-line form for TUI / `--once`.
///
/// `Ping` = mean of external anycast DNS RTTs; `GW` = default gateway.
pub fn format_line(s: &HostStats) -> String {
    let cpu = crate::fmt::fmt_cpu(s.cpu_pct);
    let mem = crate::fmt::fmt_mem_pair(s.mem_used, s.mem_total);
    let down = s
        .down_bps
        .map(crate::fmt::fmt_mbps)
        .unwrap_or_else(|| "—".into());
    let up = s
        .up_bps
        .map(crate::fmt::fmt_mbps)
        .unwrap_or_else(|| "—".into());
    let (ping, gw) = fmt_ping_pair(s);
    format!("CPU {cpu}, Memory {mem}, Downlink: {down}, Uplink: {up}, Ping: {ping}, GW: {gw}")
}

/// Width-aware host line: full labels when room, then arrows, then truncated.
pub fn format_line_for_width(s: &HostStats, width: usize) -> String {
    let full = format_line(s);
    if full.chars().count() <= width {
        return full;
    }
    let cpu = crate::fmt::fmt_cpu(s.cpu_pct);
    let mem = crate::fmt::fmt_mem_pair(s.mem_used, s.mem_total);
    let down = s
        .down_bps
        .map(crate::fmt::fmt_mbps)
        .unwrap_or_else(|| "—".into());
    let up = s
        .up_bps
        .map(crate::fmt::fmt_mbps)
        .unwrap_or_else(|| "—".into());
    let (ping, gw) = fmt_ping_pair(s);
    let compact = format!("CPU {cpu}  Mem {mem}  ↓ {down}  ↑ {up}  ping {ping}  gw {gw}");
    if compact.chars().count() <= width {
        return compact;
    }
    let tight = format!("{cpu}  {mem}  ↓{down}  ↑{up}  {ping}/{gw}");
    if tight.chars().count() <= width {
        return tight;
    }
    crate::fmt::truncate_middle(&tight, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn poll_cpu_mem_returns_finite_memory() {
        let mut m = HostMonitor::new();
        let s = m.poll_cpu_mem();
        assert!(s.mem_total > 0, "total RAM should be non-zero");
        assert!(s.mem_used <= s.mem_total);
        assert!(s.cpu_pct.is_finite());
        assert!(s.down_bps.is_none());
        assert!(s.up_bps.is_none());
        assert!(s.ping_ms.is_none());
        assert!(s.ping_gw_ms.is_none());
    }

    #[test]
    fn second_net_poll_has_rates() {
        let mut m = HostMonitor::new();
        let _ = m.poll_network();
        thread::sleep(Duration::from_millis(50));
        let s = m.poll_network();
        assert!(s.down_bps.is_some());
        assert!(s.up_bps.is_some());
        assert!(s.down_bps.unwrap().is_finite());
        assert!(s.up_bps.unwrap().is_finite());
    }

    #[test]
    fn format_line_shape() {
        let s = HostStats {
            cpu_pct: 42.6,
            mem_used: 3_338_600_448,  // ~3.11G
            mem_total: 4_294_967_296, // 4G
            down_bps: Some(68.442e6),
            up_bps: Some(75.497e6),
            ping_ms: Some(14.2),
            ping_gw_ms: Some(1.3),
        };
        let line = format_line(&s);
        assert!(line.starts_with("CPU 42.6%"), "{line}");
        assert!(line.contains("Memory "), "{line}");
        assert!(line.contains("Downlink:"), "{line}");
        assert!(line.contains("Uplink:"), "{line}");
        assert!(line.contains("Ping:"), "{line}");
        assert!(line.contains("GW:"), "{line}");
        assert!(line.contains("Mbps"), "{line}");
    }

    #[test]
    fn sparkline_and_window_stats() {
        assert_eq!(sparkline(&[], 8), "—");
        let flat = sparkline(&[10.0, 10.0, 10.0], 3);
        assert_eq!(flat.chars().count(), 3);
        // Rising series should end at a higher (or equal) bar than it starts.
        let rising = sparkline(&[1.0, 5.0, 10.0, 20.0], 4);
        let chars: Vec<char> = rising.chars().collect();
        assert_eq!(chars.len(), 4);
        let first = SPARK_BARS.iter().position(|&c| c == chars[0]).unwrap();
        let last = SPARK_BARS.iter().position(|&c| c == chars[3]).unwrap();
        assert!(last >= first, "{rising}");

        let mut m = HostMonitor::new();
        assert!(m.ping_window_stats(Duration::from_secs(300)).is_none());
        let _ = m.apply_ping(PingResult {
            external_ms: Some(10.0),
            gateway_ms: Some(1.0),
        });
        let _ = m.apply_ping(PingResult {
            external_ms: Some(30.0),
            gateway_ms: Some(1.0),
        });
        let st = m.ping_window_stats(Duration::from_secs(300)).unwrap();
        assert_eq!(st.n, 2);
        assert!((st.avg - 20.0).abs() < 1e-9);
        assert!((st.max - 30.0).abs() < 1e-9);
        assert!(m.ping_history_len() == 2);
        let hist = format_ping_history_line(&m, 80);
        assert!(hist.contains("ping "), "{hist}");
        assert!(hist.contains("5m "), "{hist}");
    }

    #[test]
    fn parse_ping_variants() {
        assert_eq!(
            parse_ping_ms("64 bytes from 1.1.1.1: icmp_seq=1 ttl=57 time=14.2 ms"),
            Some(14.2)
        );
        assert_eq!(
            parse_ping_ms("Reply from 1.1.1.1: bytes=32 time=12ms TTL=117"),
            Some(12.0)
        );
        assert_eq!(parse_ping_ms("time=0.431 ms"), Some(0.431));
        assert!(parse_ping_ms("no reply").is_none());
    }

    #[test]
    fn parse_via_gateway_line() {
        assert_eq!(
            parse_via_gateway("default via 192.168.1.1 dev eth0 proto dhcp src 192.168.1.10"),
            Some("192.168.1.1".into())
        );
        assert!(parse_via_gateway("unreachable default").is_none());
    }

    #[test]
    fn loopback_names() {
        assert!(is_loopback("lo"));
        assert!(is_loopback("lo0"));
        assert!(is_loopback("LO"));
        assert!(!is_loopback("eth0"));
        assert!(!is_loopback("en0"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn gateway_from_proc_is_ipv4_or_none() {
        // Sandboxes may lack a default route; when present it must look like IPv4.
        if let Some(gw) = gateway_from_proc_route() {
            let parts: Vec<_> = gw.split('.').collect();
            assert_eq!(parts.len(), 4, "{gw}");
            for p in parts {
                let _ = p.parse::<u8>().expect("octet");
            }
        }
    }
}
