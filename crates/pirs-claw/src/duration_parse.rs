//! Parse human durations for schedule `--in` / `--every` (e.g. `30s`, `5m`, `2h`, `1d`).

/// Parse a duration string to seconds.
/// Accepts plain integers (seconds) or `{n}s|m|h|d` (case-insensitive).
pub fn parse_duration_secs(s: &str) -> anyhow::Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("empty duration");
    }
    if let Ok(n) = s.parse::<u64>() {
        return Ok(n);
    }
    let s = s.to_ascii_lowercase();
    let (num, unit) = s.split_at(
        s.find(|c: char| c.is_ascii_alphabetic())
            .ok_or_else(|| anyhow::anyhow!("invalid duration {s:?}"))?,
    );
    let n: u64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration number in {s:?}"))?;
    let secs = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => n,
        "m" | "min" | "mins" | "minute" | "minutes" => n.saturating_mul(60),
        "h" | "hr" | "hrs" | "hour" | "hours" => n.saturating_mul(3600),
        "d" | "day" | "days" => n.saturating_mul(86400),
        _ => anyhow::bail!("unknown duration unit {unit:?} in {s:?}"),
    };
    Ok(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units() {
        assert_eq!(parse_duration_secs("30").unwrap(), 30);
        assert_eq!(parse_duration_secs("30s").unwrap(), 30);
        assert_eq!(parse_duration_secs("5m").unwrap(), 300);
        assert_eq!(parse_duration_secs("2h").unwrap(), 7200);
        assert_eq!(parse_duration_secs("1d").unwrap(), 86400);
        assert_eq!(parse_duration_secs("1H").unwrap(), 3600);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_duration_secs("").is_err());
        assert!(parse_duration_secs("xx").is_err());
        assert!(parse_duration_secs("5w").is_err());
    }
}
