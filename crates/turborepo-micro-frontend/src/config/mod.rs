/// Module for parsing micro-frontend configuration from JSON. Not intended for
/// direct use.
mod parse;
/// Module for validating data received from parse module
mod validate;

pub use validate::{Application, Config, Host};
