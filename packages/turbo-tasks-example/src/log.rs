use turbo_tasks::Task;

use crate::math::I32ValueRef;

#[turbo_tasks::function]
pub async fn log(a: I32ValueRef) -> I32ValueRef {
    println!("{}", a.get().value);
    Task::side_effect();
    a
}
