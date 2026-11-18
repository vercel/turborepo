/// `amount` units of a currency at `rate` cents per unit.
pub fn to_cents(amount: u64, rate: u64) -> u64 {
    amount * rate
}

#[cfg(test)]
mod tests {
    #[test]
    fn converts() {
        assert_eq!(super::to_cents(2, 108), 216);
    }
}
