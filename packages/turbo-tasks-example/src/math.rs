use std::time::Duration;

#[turbo_tasks::function]
pub async fn add(a: I32ValueRef, b: I32ValueRef) -> I32ValueRef {
    let a = a.get().value;
    let b = b.get().value;
    println!("{} + {} = ...", a, b);
    async_std::task::sleep(Duration::from_secs(1)).await;
    println!("{} + {} = {}", a, b, a + b);
    I32ValueRef::new(a + b)
}

#[turbo_tasks::value]
pub struct I32Value {
    pub value: i32,
}

#[turbo_tasks::value_impl]
impl I32Value {
    #[turbo_tasks::constructor]
    pub fn new(value: i32) -> Self {
        Self { value }
    }
}
