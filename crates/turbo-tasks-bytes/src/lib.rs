pub mod bytes;
pub mod stream;

use turbo_tasks::Vc;

pub use crate::{
    bytes::Bytes,
    stream::{Stream, StreamRead},
};

pub fn register() {
    turbo_tasks::register();
    include!(concat!(env!("OUT_DIR"), "/register.rs"));
}
