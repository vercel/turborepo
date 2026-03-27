use math_core::mean;

pub fn variance(values: &[f64]) -> Option<f64> {
    let m = mean(values)?;
    let sum_sq: f64 = values.iter().map(|v| (v - m).powi(2)).sum();
    Some(sum_sq / values.len() as f64)
}

pub fn std_dev(values: &[f64]) -> Option<f64> {
    variance(values).map(f64::sqrt)
}

pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        mean(&[sorted[mid - 1], sorted[mid]])
    } else {
        Some(sorted[mid])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_variance() {
        let v = variance(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]).unwrap();
        assert!((v - 4.0).abs() < 1e-10);
    }

    #[test]
    fn computes_std_dev() {
        let sd = std_dev(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]).unwrap();
        assert!((sd - 2.0).abs() < 1e-10);
    }

    #[test]
    fn computes_median_odd() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
    }

    #[test]
    fn computes_median_even() {
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(variance(&[]), None);
        assert_eq!(std_dev(&[]), None);
        assert_eq!(median(&[]), None);
    }
}
