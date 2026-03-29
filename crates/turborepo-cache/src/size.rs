use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SizeParseError {
    #[error("empty size string")]
    Empty,
    #[error("invalid size \"{0}\": expected a number followed by a unit (B, KB, MB, GB, TB)")]
    InvalidFormat(String),
    #[error("invalid size \"{0}\": unknown unit '{1}', expected B, KB, MB, GB, or TB")]
    UnknownUnit(String, String),
    #[error("invalid size \"{0}\": numeric part is not a valid number")]
    InvalidNumber(String),
    #[error("invalid size \"{0}\": value overflows maximum representable size")]
    Overflow(String),
}

/// Parses a human-readable byte size string into a byte count.
///
/// Supported formats (case-insensitive):
/// - `"0"` — zero bytes (disabled)
/// - `"1024B"` — 1024 bytes
/// - `"500KB"` — 500 kilobytes (500 * 1024)
/// - `"10MB"` — 10 megabytes
/// - `"5GB"` — 5 gigabytes
/// - `"1TB"` — 1 terabyte
///
/// Plain integers without a unit are treated as **bytes**.
pub fn parse_human_size(input: &str) -> Result<u64, SizeParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(SizeParseError::Empty);
    }

    let upper = input.to_ascii_uppercase();

    if upper == "0" {
        return Ok(0);
    }

    // Find where the numeric part ends and the unit begins
    let unit_start = upper
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(upper.len());

    let num_part = &upper[..unit_start];
    let unit_part = &upper[unit_start..];

    if num_part.is_empty() {
        return Err(SizeParseError::InvalidNumber(input.to_owned()));
    }

    let multiplier: u64 = match unit_part {
        "" | "B" => 1,
        "KB" => 1024,
        "MB" => 1024 * 1024,
        "GB" => 1024 * 1024 * 1024,
        "TB" => 1024 * 1024 * 1024 * 1024,
        _ => {
            return Err(SizeParseError::UnknownUnit(
                input.to_owned(),
                unit_part.to_owned(),
            ));
        }
    };

    // Use exact integer arithmetic when possible, fall back to f64 for fractions
    if num_part.contains('.') {
        let n: f64 = num_part
            .parse()
            .map_err(|_| SizeParseError::InvalidNumber(input.to_owned()))?;

        if !n.is_finite() || n < 0.0 {
            return Err(SizeParseError::InvalidNumber(input.to_owned()));
        }

        let result = n * multiplier as f64;
        if result > u64::MAX as f64 {
            return Err(SizeParseError::Overflow(input.to_owned()));
        }

        Ok(result as u64)
    } else {
        let n: u64 = num_part
            .parse()
            .map_err(|_| SizeParseError::InvalidNumber(input.to_owned()))?;

        n.checked_mul(multiplier)
            .ok_or_else(|| SizeParseError::Overflow(input.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero() {
        assert_eq!(parse_human_size("0").unwrap(), 0);
    }

    #[test]
    fn test_bytes() {
        assert_eq!(parse_human_size("1024B").unwrap(), 1024);
    }

    #[test]
    fn test_kilobytes() {
        assert_eq!(parse_human_size("500KB").unwrap(), 500 * 1024);
    }

    #[test]
    fn test_megabytes() {
        assert_eq!(parse_human_size("10MB").unwrap(), 10 * 1024 * 1024);
    }

    #[test]
    fn test_gigabytes() {
        assert_eq!(parse_human_size("5GB").unwrap(), 5 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_terabytes() {
        assert_eq!(parse_human_size("1TB").unwrap(), 1024 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_plain_integer_treated_as_bytes() {
        assert_eq!(parse_human_size("4096").unwrap(), 4096);
    }

    #[test]
    fn test_case_insensitive() {
        assert_eq!(parse_human_size("5gb").unwrap(), 5 * 1024 * 1024 * 1024);
        assert_eq!(parse_human_size("5Gb").unwrap(), 5 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_whitespace_trimmed() {
        assert_eq!(
            parse_human_size("  10GB  ").unwrap(),
            10 * 1024 * 1024 * 1024
        );
    }

    #[test]
    fn test_fractional() {
        assert_eq!(
            parse_human_size("1.5GB").unwrap(),
            (1.5 * 1024.0 * 1024.0 * 1024.0) as u64
        );
    }

    #[test]
    fn test_empty() {
        assert_eq!(parse_human_size(""), Err(SizeParseError::Empty));
    }

    #[test]
    fn test_unknown_unit() {
        assert!(matches!(
            parse_human_size("10PB"),
            Err(SizeParseError::UnknownUnit(_, _))
        ));
    }

    #[test]
    fn test_invalid_number() {
        assert!(matches!(
            parse_human_size("abcGB"),
            Err(SizeParseError::InvalidNumber(_))
        ));
    }

    #[test]
    fn test_negative_rejected() {
        assert!(matches!(
            parse_human_size("-5GB"),
            Err(SizeParseError::InvalidNumber(_))
        ));
    }

    #[test]
    fn test_nan_rejected() {
        assert!(matches!(
            parse_human_size("NaNGB"),
            Err(SizeParseError::InvalidNumber(_))
        ));
    }

    #[test]
    fn test_infinity_rejected() {
        assert!(matches!(
            parse_human_size("InfinityGB"),
            Err(SizeParseError::InvalidNumber(_))
        ));
    }

    #[test]
    fn test_overflow() {
        assert!(matches!(
            parse_human_size("99999999999999999TB"),
            Err(SizeParseError::Overflow(_))
        ));
    }
}
