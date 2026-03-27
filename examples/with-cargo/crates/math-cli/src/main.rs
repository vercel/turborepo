use math_core::{mean, moving_average};
use math_ext::{median, std_dev};
use math_fmt::{format_mean, format_series};

fn main() {
    let values = [3.0, 4.0, 8.0, 9.0];
    let avg = mean(&values).unwrap_or_default();
    let trend = moving_average(&values, 2);
    let med = median(&values).unwrap_or_default();
    let sd = std_dev(&values).unwrap_or_default();

    println!("values: {}", format_series(&values, 1));
    println!("mean: {}", format_mean(&values, 2));
    println!("median: {med:.2}");
    println!("std dev: {sd:.2}");
    println!("raw mean: {avg:.2}");
    println!("moving average (window=2): {trend:?}");
}
