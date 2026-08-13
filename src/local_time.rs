use chrono::{DateTime, FixedOffset, Local, Utc};

pub fn format_timestamp_at_offset(
    timestamp_ms: Option<u64>,
    offset: FixedOffset,
) -> Option<String> {
    let timestamp_ms = i64::try_from(timestamp_ms?).ok()?;
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms).map(|timestamp| {
        timestamp
            .with_timezone(&offset)
            .format("%d.%m.%Y %H:%M")
            .to_string()
    })
}

pub fn format_local_timestamp(timestamp_ms: Option<u64>) -> Option<String> {
    let timestamp_ms = i64::try_from(timestamp_ms?).ok()?;
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms).map(|timestamp| {
        timestamp
            .with_timezone(&Local)
            .format("%d.%m.%Y %H:%M")
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::{format_local_timestamp, format_timestamp_at_offset};
    use chrono::FixedOffset;

    #[test]
    fn fixed_epoch_formats_in_the_supplied_offset() {
        let offset = FixedOffset::east_opt(3 * 60 * 60).unwrap();

        assert_eq!(
            format_timestamp_at_offset(Some(1_786_638_720_000), offset).as_deref(),
            Some("13.08.2026 19:32")
        );
    }

    #[test]
    fn missing_and_out_of_range_timestamps_stay_blank() {
        let offset = FixedOffset::east_opt(0).unwrap();

        assert_eq!(format_timestamp_at_offset(None, offset), None);
        assert_eq!(format_timestamp_at_offset(Some(u64::MAX), offset), None);
        assert_eq!(format_local_timestamp(None), None);
    }

    #[test]
    fn west_offset_can_cross_the_local_calendar_boundary() {
        let offset = FixedOffset::west_opt(60 * 60).unwrap();

        assert_eq!(
            format_timestamp_at_offset(Some(0), offset).as_deref(),
            Some("31.12.1969 23:00")
        );
    }

    #[test]
    fn every_component_is_two_digits_and_seconds_are_omitted() {
        let offset = FixedOffset::east_opt(0).unwrap();

        assert_eq!(
            format_timestamp_at_offset(Some(1_767_323_040_000), offset).as_deref(),
            Some("02.01.2026 03:04")
        );
    }
}
