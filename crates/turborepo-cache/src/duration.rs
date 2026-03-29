use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DurationParseError {
    #[error("empty duration string")]
    Empty,
    #[error("invalid duration \"{0}\": expected a number followed by a unit (s, m, h, d, w)")]
    InvalidFormat(String),
    #[error("invalid duration \"{0}\": unknown unit '{1}', expected s, m, h, d, or w")]
    UnknownUnit(String, char),
    #[error("invalid duration \"{0}\": numeric part is not a valid integer")]
    InvalidNumber(String),
    #[error("invalid duration \"{0}\": value overflows maximum representable duration")]
    Overflow(String),
}

/// Parses a human-readable duration string into a `std::time::Duration`.
///
/// Supported formats:
/// - `"0"` — zero duration (disabled)
/// - `"30s"` — 30 seconds
/// - `"5m"` — 5 minutes
/// - `"24h"` — 24 hours
/// - `"7d"` — 7 days
/// - `"2w"` — 2 weeks
///
/// Plain integers without a unit are treated as **days** for convenience
/// (e.g. `"7"` = 7 days).
pub fn parse_human_duration(input: &str) -> Result<Duration, DurationParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(DurationParseError::Empty);
    }

    if input == "0" {
        return Ok(Duration::ZERO);
    }

    let last = input.chars().last().unwrap();

    if last.is_ascii_digit() {
        // Plain integer — treat as days
        let n: u64 = input
            .parse()
            .map_err(|_| DurationParseError::InvalidNumber(input.to_owned()))?;
        let secs = n
            .checked_mul(86_400)
            .ok_or_else(|| DurationParseError::Overflow(input.to_owned()))?;
        return Ok(Duration::from_secs(secs));
    }

    let (num_part, unit) = input.split_at(input.len() - 1);
    let unit_char = unit.chars().next().unwrap();

    let n: u64 = num_part
        .parse()
        .map_err(|_| DurationParseError::InvalidNumber(input.to_owned()))?;

    let secs = match unit_char {
        's' => Some(n),
        'm' => n.checked_mul(60),
        'h' => n.checked_mul(3_600),
        'd' => n.checked_mul(86_400),
        'w' => n.checked_mul(604_800),
        _ => return Err(DurationParseError::UnknownUnit(input.to_owned(), unit_char)),
    }
    .ok_or_else(|| DurationParseError::Overflow(input.to_owned()))?;

    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero() {
        assert_eq!(parse_human_duration("0").unwrap(), Duration::ZERO);
    }

    #[test]
    fn test_seconds() {
        assert_eq!(
            parse_human_duration("30s").unwrap(),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn test_minutes() {
        assert_eq!(
            parse_human_duration("5m").unwrap(),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn test_hours() {
        assert_eq!(
            parse_human_duration("24h").unwrap(),
            Duration::from_secs(86_400)
        );
    }

    #[test]
    fn test_days() {
        assert_eq!(
            parse_human_duration("7d").unwrap(),
            Duration::from_secs(604_800)
        );
    }

    #[test]
    fn test_weeks() {
        assert_eq!(
            parse_human_duration("2w").unwrap(),
            Duration::from_secs(1_209_600)
        );
    }

    #[test]
    fn test_plain_integer_treated_as_days() {
        assert_eq!(
            parse_human_duration("7").unwrap(),
            Duration::from_secs(604_800)
        );
    }

    #[test]
    fn test_whitespace_trimmed() {
        assert_eq!(
            parse_human_duration("  7d  ").unwrap(),
            Duration::from_secs(604_800)
        );
    }

    #[test]
    fn test_empty() {
        assert_eq!(parse_human_duration(""), Err(DurationParseError::Empty));
    }

    #[test]
    fn test_unknown_unit() {
        assert!(matches!(
            parse_human_duration("7x"),
            Err(DurationParseError::UnknownUnit(_, 'x'))
        ));
    }

    #[test]
    fn test_invalid_number() {
        assert!(matches!(
            parse_human_duration("abcd"),
            Err(DurationParseError::InvalidNumber(_))
        ));
    }

    #[test]
    fn test_one_second() {
        assert_eq!(parse_human_duration("1s").unwrap(), Duration::from_secs(1));
    }

    #[test]
    fn test_overflow_large_weeks() {
        assert!(matches!(
            parse_human_duration("99999999999999999w"),
            Err(DurationParseError::Overflow(_))
        ));
    }

    #[test]
    fn test_overflow_bare_integer() {
        assert!(matches!(
            parse_human_duration("999999999999999999"),
            Err(DurationParseError::Overflow(_))
        ));
    }
}
