#![feature(cow_is_borrowed)]
#![feature(assert_matches)]
#![deny(clippy::all)]
//! Turborepo's library for authenticating with the Vercel API.
//! Handles logging into Vercel, verifying SSO, and storing the token.

mod auth;
mod error;
mod login_server;
mod ui;

pub use auth::*;
pub use error::Error;
pub use login_server::*;

/// Token is the result of a successful login. It contains the token string and
/// a boolean indicating whether the token already existed on the filesystem.
#[derive(Debug)]
pub enum Token {
    Existing(String),
    New(String),
}
impl Token {
    pub fn into_inner(self) -> String {
        match self {
            Self::Existing(s) => s,
            Self::New(s) => s,
        }
    }
}
