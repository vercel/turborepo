fn main() {
    println!("MY_SECRET_TOKEN={:?}", std::env::var("MY_SECRET_TOKEN").ok());
}
