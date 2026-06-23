//! Small pure formatting helpers shared between `dump`, `meta` rendering, and
//! the TUI row formatter. Keep this module dependency-free.

/// Placeholder for an absent text field.
pub const DASH: &str = "—";

pub fn opt_str(s: &Option<String>) -> &str {
    match s {
        Some(v) if !v.is_empty() => v.as_str(),
        _ => DASH,
    }
}

pub fn format_duration(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

pub fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    let b = bytes as f64;
    if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.0} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}
