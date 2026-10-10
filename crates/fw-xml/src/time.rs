//! Times are integer milliseconds in memory and "seconds with three decimals"
//! in the file. Conversions are exact ports of Rocksmith2014.NET's so that
//! parsing truncates (never rounds) extra decimals, as the game's tools do.

/// Milliseconds -> "s.mmm".
pub fn to_string(ms: i32) -> String {
    let neg = ms < 0;
    let digits = ms.unsigned_abs().to_string();
    let body = match digits.len() {
        1 => format!("0.00{digits}"),
        2 => format!("0.0{digits}"),
        3 => format!("0.{digits}"),
        n => format!("{}.{}", &digits[..n - 3], &digits[n - 3..]),
    };
    if neg { format!("-{body}") } else { body }
}

/// "s.mmm" (any number of decimals) -> milliseconds, truncating past 3 decimals.
pub fn parse(s: &str) -> Option<i32> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let mut ms = 0i64;
    for (i, c) in frac.chars().take(3).enumerate() {
        ms += c.to_digit(10)? as i64 * [100, 10, 1][i];
    }
    if !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let v = whole.checked_mul(1000)?.checked_add(ms)?;
    i32::try_from(if neg { -v } else { v }).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(to_string(0), "0.000");
        assert_eq!(to_string(5), "0.005");
        assert_eq!(to_string(45), "0.045");
        assert_eq!(to_string(345), "0.345");
        assert_eq!(to_string(12345), "12.345");
        assert_eq!(to_string(-500), "-0.500");
    }

    #[test]
    fn parses_and_truncates() {
        assert_eq!(parse("1.5"), Some(1500));
        assert_eq!(parse("1.2349"), Some(1234));
        assert_eq!(parse("7"), Some(7000));
        assert_eq!(parse("-0.5"), Some(-500));
        assert_eq!(parse(".25"), Some(250));
        assert_eq!(parse("abc"), None);
        assert_eq!(parse("1.2x"), None);
    }
}
