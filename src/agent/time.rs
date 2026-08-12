use serde_json::Value;

pub(super) fn record_timestamp_ms(record: &Value) -> Option<u64> {
    if let Some(value) = record.get("timestamp_ms").and_then(Value::as_u64) {
        return Some(value);
    }
    let timestamp = record.get("timestamp")?;
    if let Some(value) = timestamp.as_u64() {
        return if value < 1_000_000_000_000 {
            value.checked_mul(1_000)
        } else {
            Some(value)
        };
    }
    parse_rfc3339_millis(timestamp.as_str()?)
}

fn parse_rfc3339_millis(value: &str) -> Option<u64> {
    if value.len() < 20 || value.as_bytes().get(4) != Some(&b'-') {
        return None;
    }
    let year = parse_i64(&value[0..4])?;
    let month = parse_i64(&value[5..7])?;
    let day = parse_i64(&value[8..10])?;
    let hour = parse_i64(&value[11..13])?;
    let minute = parse_i64(&value[14..16])?;
    let second = parse_i64(&value[17..19])?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }

    let suffix = &value[19..];
    let (fraction, zone) = if let Some(fraction) = suffix.strip_prefix('.') {
        let split = fraction.find(['Z', '+', '-'])?;
        (&fraction[..split], &fraction[split..])
    } else {
        ("", suffix)
    };
    let millis = fraction
        .chars()
        .take(3)
        .chain(std::iter::repeat('0'))
        .take(3)
        .collect::<String>()
        .parse::<i64>()
        .ok()?;
    let offset_seconds = if zone == "Z" {
        0
    } else {
        let sign = match zone.as_bytes().first()? {
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        if zone.len() != 6 || zone.as_bytes().get(3) != Some(&b':') {
            return None;
        }
        sign * (parse_i64(&zone[1..3])? * 3_600 + parse_i64(&zone[4..6])? * 60)
    };
    let days = days_from_civil(year, month, day);
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?
        .checked_sub(offset_seconds)?;
    u64::try_from(seconds.checked_mul(1_000)?.checked_add(millis)?).ok()
}

fn parse_i64(value: &str) -> Option<i64> {
    value.parse().ok()
}

fn days_from_civil(mut year: i64, month: i64, day: i64) -> i64 {
    year -= i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::parse_rfc3339_millis;

    #[test]
    fn parses_utc_fraction_and_offset() {
        assert_eq!(
            parse_rfc3339_millis("2026-08-12T10:00:00Z"),
            Some(1_786_528_800_000)
        );
        assert_eq!(
            parse_rfc3339_millis("2026-08-12T13:00:00.125+03:00"),
            Some(1_786_528_800_125)
        );
    }
}
