pub fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

pub fn moving_average(values: &[f64], window: usize) -> Vec<f64> {
    if window == 0 || values.len() < window {
        return Vec::new();
    }

    values
        .windows(window)
        .map(|slice| slice.iter().sum::<f64>() / window as f64)
        .collect()
}
