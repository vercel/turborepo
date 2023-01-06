#![feature(arbitrary_self_types)]
#![feature(async_fn_in_trait)]

// mod other;
pub mod val;

pub use val::run;

pub fn register() {
    turbo_tasks::register();
    include!(concat!(env!("OUT_DIR"), "/register.rs"));
}
