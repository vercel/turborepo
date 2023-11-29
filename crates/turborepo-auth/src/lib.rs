#![feature(cow_is_borrowed)]
#![deny(clippy::all)]
//! Turborepo's library for authenticating with the Vercel API.
//! Handles logging into Vercel, verifying SSO, and storing the token.

mod auth;
mod error;
mod server;
mod ui;

pub use auth::*;
pub use error::Error;
pub use server::*;
