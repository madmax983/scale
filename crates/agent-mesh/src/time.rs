//! Time helpers. The mesh uses Unix seconds in UTC.

/// Formats Unix seconds as RFC 3339 UTC, for example `2026-10-13T16:40:00Z`.
#[must_use]
pub fn fmt_utc(secs: u64) -> String {
    let (y, m, d) = civil_from_days(secs / 86_400);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

/// Converts days since 1970-01-01 to (year, month, day).
/// Algorithm: H. Hinnant, "chrono-Compatible Low-Level Date Algorithms".
const fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_times() {
        assert_eq!(fmt_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(fmt_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(fmt_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(fmt_utc(4_102_444_799), "2099-12-31T23:59:59Z");
    }
}
