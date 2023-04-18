pub mod module_type;
mod process;
mod source;

pub fn register() {
    turbo_tasks::register();
    turbo_tasks_fs::register();
    turbopack_core::register();
    turbopack_ecmascript::register();
    turbopack_static::register();
    include!(concat!(env!("OUT_DIR"), "/register.rs"));
}
