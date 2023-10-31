#![feature(cow_is_borrowed)]
#![deny(clippy::all)]

mod auth;
mod error;
mod mocks;
mod server;
mod token;
mod ui;

pub use auth::*;
pub use error::Error;
pub use server::*;
pub use token::*;

pub const AUTH_FILE_NAME: &str = "auth.json";
pub const TURBOREPO_CONFIG_DIR: &str = "turborepo";
