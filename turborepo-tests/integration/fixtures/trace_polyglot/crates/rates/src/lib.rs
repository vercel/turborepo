//! Exchange rates, in cents per unit, from `shared/rates.json`.

mod convert;

pub use convert::to_cents;

/// The rates file, as shipped.
pub const RATES: &str = include_str!("../../../shared/rates.json");

/// The rate of `code`, if the file lists it.
pub fn rate(code: &str) -> Option<u64> {
    let key = format!("\"{code}\":");
    let start = RATES.find(&key)? + key.len();
    let rest = RATES[start..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit())?;
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn knows_the_euro() {
        assert!(super::rate("EUR").is_some());
    }
}
