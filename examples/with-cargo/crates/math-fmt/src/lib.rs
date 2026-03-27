use math_core::mean;

pub fn format_mean(values: &[f64], precision: usize) -> String {
    match mean(values) {
        Some(m) => format!("{m:.precision$}"),
        None => "N/A".to_string(),
    }
}

pub fn format_series(values: &[f64], precision: usize) -> String {
    values
        .iter()
        .map(|v| format!("{v:.precision$}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_mean() {
        assert_eq!(format_mean(&[2.0, 4.0], 1), "3.0");
    }

    #[test]
    fn formats_empty_mean() {
        assert_eq!(format_mean(&[], 2), "N/A");
    }

    #[test]
    fn formats_series() {
        assert_eq!(format_series(&[1.5, 2.5], 1), "1.5, 2.5");
    }
}
