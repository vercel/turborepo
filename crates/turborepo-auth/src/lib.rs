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

pub const TURBOREPO_AUTH_FILE_NAME: &str = "auth.json";
pub const TURBOREPO_LEGACY_AUTH_FILE_NAME: &str = "config.json";
pub const TURBOREPO_CONFIG_DIR: &str = "turborepo";

pub const DEFAULT_LOGIN_URL: &str = "https://vercel.com";
pub const DEFAULT_API_URL: &str = "https://vercel.com/api";
