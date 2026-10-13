//! Time helpers. The mesh uses Unix seconds in UTC.

/// Formats Unix seconds as RFC 3339 UTC, for example `2026-10-13T16:40:00Z`.
#[must_use]
pub fn fmt_utc(secs: u64) -> String {
    let _ = secs;
    todo!("RED")
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
