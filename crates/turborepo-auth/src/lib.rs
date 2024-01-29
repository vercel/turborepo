#![feature(cow_is_borrowed)]
#![deny(clippy::all)]
//! Turborepo's library for authenticating with the Vercel API.
//! Handles logging into Vercel, verifying SSO, and storing the token.

mod auth;
mod error;
mod server;
// mod sso;
mod ui;

pub use auth::*;
use error::AuthError;
pub use error::Error;
pub use server::*;
// use sso::authenticate_via_sso;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn login(api: Option<String>, use_sso: bool) -> Result<JsValue, JsValue> {
    let result = if use_sso {
        todo!()
        // authenticate_via_sso(api.as_deref()).map_err(|e| e.to_string())
    } else {
        authenticate(api.as_deref()).map_err(|e| e.to_string())
    };

    match result {
        Ok(token) => Ok(JsValue::from_str(&token)),
        Err(e) => Err(JsValue::from_str(&e)),
    }
}
