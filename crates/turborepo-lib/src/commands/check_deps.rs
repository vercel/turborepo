use crate::cli;

pub async fn run() -> Result<i32, cli::Error> {
    println!("hello world");
    Ok(0)
}
