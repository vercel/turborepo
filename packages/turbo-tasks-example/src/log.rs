use turbo_tasks::Task;

use crate::math::I32ValueRef;

#[turbo_tasks::function]
pub async fn log(a: I32ValueRef, options: LoggingOptionsRef) {
    let options = options.await;
    let a = a.await;
    println!("{}: {}", options.name, a.value);
    Task::side_effect();
}

#[turbo_tasks::value]
pub struct LoggingOptions {
    name: String,
}

#[turbo_tasks::value_impl]
impl LoggingOptions {
    #[turbo_tasks::constructor(compare)]
    pub fn new(name: String) -> Self {
        Self { name }
    }
}
