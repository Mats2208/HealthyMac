//! Human-readable formatting, in Spanish.

use std::time::SystemTime;

/// Formats bytes in decimal units, matching Finder ("1,2 GB").
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    let text = if value >= 100.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}").replace('.', ",")
    };
    format!("{text} {}", UNITS[unit])
}

/// Formats memory in binary units, as About This Mac and Activity Monitor do, so that 48 GiB of RAM
/// reads "48 GB" rather than the "51,5 GB" that [`human_size`] would give.
pub fn human_memory(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        let gib = value / GIB;
        if (gib - gib.round()).abs() < 0.05 {
            format!("{gib:.0} GB")
        } else {
            format!("{gib:.1} GB").replace('.', ",")
        }
    } else {
        format!("{:.0} MB", value / MIB)
    }
}

/// Formats how long ago `then` was, relative to `now` ("hace 3 días").
pub fn human_age(then: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(then).map(|d| d.as_secs()).unwrap_or(0);
    let plural = |n: u64, one: &str, many: &str| {
        if n == 1 {
            format!("hace 1 {one}")
        } else {
            format!("hace {n} {many}")
        }
    };
    match secs {
        0..=59 => "ahora mismo".to_string(),
        60..=3_599 => plural(secs / 60, "minuto", "minutos"),
        3_600..=86_399 => plural(secs / 3_600, "hora", "horas"),
        86_400..=2_591_999 => plural(secs / 86_400, "día", "días"),
        2_592_000..=31_535_999 => plural(secs / 2_592_000, "mes", "meses"),
        _ => plural(secs / 31_536_000, "año", "años"),
    }
}

/// Whole days between `then` and `now`, or `None` if `then` is in the future.
pub fn age_days(then: SystemTime, now: SystemTime) -> Option<u64> {
    now.duration_since(then).ok().map(|d| d.as_secs() / 86_400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1_000), "1,0 KB");
        assert_eq!(human_size(1_500_000), "1,5 MB");
        assert_eq!(human_size(245_000_000_000), "245 GB");
    }

    #[test]
    fn memory_uses_binary_units() {
        assert_eq!(human_memory(48 * 1024 * 1024 * 1024), "48 GB");
        assert_eq!(human_memory(40_200_896_512), "37,4 GB");
        assert_eq!(human_memory(512 * 1024 * 1024), "512 MB");
    }

    #[test]
    fn ages() {
        let now = SystemTime::now();
        let ago = |secs| now - Duration::from_secs(secs);
        assert_eq!(human_age(ago(5), now), "ahora mismo");
        assert_eq!(human_age(ago(60), now), "hace 1 minuto");
        assert_eq!(human_age(ago(86_400 * 3), now), "hace 3 días");
        assert_eq!(human_age(ago(31_536_000 * 2), now), "hace 2 años");
        assert_eq!(human_age(now + Duration::from_secs(50), now), "ahora mismo");
    }
}
