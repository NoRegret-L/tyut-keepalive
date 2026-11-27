//! Presentation/storage wall-clock formats, independent of monotonic scheduling.
//! Keep the legacy local-date and RFC 3339 contracts while using a maintained library.
use jiff::Zoned;

pub(crate) fn local_datetime(now: &Zoned) -> String {
    now.strftime("%Y-%m-%d %H:%M:%S").to_string()
}

pub(crate) fn local_date(now: &Zoned) -> String {
    now.strftime("%Y-%m-%d").to_string()
}

pub(crate) fn rfc3339(now: &Zoned) -> String {
    // Do not serialize Zoned directly: its zone annotation would change JSONL format.
    now.strftime("%Y-%m-%dT%H:%M:%S%.f%:z").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_formats_preserve_date_offset_and_fraction() {
        let now: Zoned = "2026-11-27T23:59:59.123456+08:00[+08:00]".parse().unwrap();
        assert_eq!(local_datetime(&now), "2026-11-27 23:59:59");
        assert_eq!(local_date(&now), "2026-11-27");
        assert_eq!(rfc3339(&now), "2026-11-27T23:59:59.123456+08:00");
    }

    #[test]
    fn no_fraction_and_negative_offset_remain_rfc3339() {
        let now: Zoned = "2026-11-28T00:00:00-08:00[-08:00]".parse().unwrap();
        assert_eq!(local_date(&now), "2026-11-28");
        assert_eq!(rfc3339(&now), "2026-11-28T00:00:00-08:00");
    }
}
