use turbo_tasks::Task;

use crate::math::I32ValueRef;

#[turbo_tasks::function]
pub async fn log(a: I32ValueRef, options: LoggingOptionsRef) {
    println!("{}: {}", options.get().name, a.get().value);
    Task::side_effect();
}

#[turbo_tasks::value]
pub struct LoggingOptions {
    name: String,
}

#[turbo_tasks::value_impl]
impl LoggingOptions {
    #[turbo_tasks::constructor(!intern)]
    pub fn new(name: String) -> Self {
        Self { name }
    }
}
