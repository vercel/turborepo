use std::time::Duration;

#[turbo_tasks::function]
pub async fn add(a: I32ValueRef, b: I32ValueRef) -> I32ValueRef {
    let a = a.get().value;
    let b = b.get().value;
    println!("{} + {} = ...", a, b);
    async_std::task::sleep(Duration::from_millis(500)).await;
    println!("{} + {} = {}", a, b, a + b);
    I32ValueRef::new(a + b)
}

#[turbo_tasks::function]
pub async fn max_new(a: I32ValueRef, b: I32ValueRef) -> I32ValueRef {
    let a = a.get().value;
    let b = b.get().value;
    println!("max({}, {}) = ...", a, b);
    async_std::task::sleep(Duration::from_millis(500)).await;
    let max = if a > b { a } else { b };
    println!("max({}, {}) = {}", a, b, max);
    I32ValueRef::new(max)
}

#[turbo_tasks::function]
pub async fn max_reuse(a_ref: I32ValueRef, b_ref: I32ValueRef) -> I32ValueRef {
    let a = a_ref.get().value;
    let b = b_ref.get().value;
    println!("max({}, {}) = ...", a, b);
    async_std::task::sleep(Duration::from_millis(500)).await;
    println!("max({}, {}) = {}", a, b, a + b);
    if a > b {
        a_ref
    } else {
        b_ref
    }
}

#[turbo_tasks::value]
pub struct I32Value {
    pub value: i32,
}

#[turbo_tasks::value_impl]
impl I32Value {
    #[turbo_tasks::constructor(compare: is)]
    pub fn new(value: i32) -> Self {
        Self { value }
    }

    pub fn is(&self, value: &i32) -> bool {
        println!("compared I32Value {} == {}", self.value, *value);
        self.value == *value
    }
}
