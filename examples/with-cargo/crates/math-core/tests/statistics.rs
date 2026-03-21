use math_core::{mean, moving_average};

#[test]
fn computes_mean_for_non_empty_values() {
    assert_eq!(mean(&[2.0, 4.0, 6.0]), Some(4.0));
}

#[test]
fn computes_moving_average() {
    assert_eq!(moving_average(&[1.0, 3.0, 5.0, 7.0], 2), vec![2.0, 4.0, 6.0]);
}
