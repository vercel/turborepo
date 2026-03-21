use math_core::{mean, moving_average};

fn main() {
    let values = [3.0, 4.0, 8.0, 9.0];
    let avg = mean(&values).unwrap_or_default();
    let trend = moving_average(&values, 2);

    println!("values: {values:?}");
    println!("mean: {avg:.2}");
    println!("moving average (window=2): {trend:?}");
}
