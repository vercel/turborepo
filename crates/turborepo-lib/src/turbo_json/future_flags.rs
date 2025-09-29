//! Future flags for enabling experimental or upcoming features
//!
//! This module contains the `FutureFlags` structure which allows users to
//! opt-in to experimental features before they become the default behavior.
//!
//! ## Usage
//!
//! Future flags can be configured in the root `turbo.json`:
//!
//! ```json
//! {
//!   "futureFlags": {
//!     "turboExtends": true
//!   }
//! }
//! ```
//!
//! Note: Future flags are only allowed in the root `turbo.json` and will cause
//! an error if specified in workspace packages.

use biome_deserialize_macros::Deserializable;
use serde::Serialize;
use struct_iterable::Iterable;

/// Future flags configuration for experimental features
///
/// Each flag represents an experimental feature that can be enabled
/// before it becomes the default behavior in a future version.
#[derive(Serialize, Default, Debug, Copy, Clone, Iterable, Deserializable, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[deserializable()]
pub struct FutureFlags {
    /// Enable `$TURBO_EXTENDS$`
    ///
    /// When enabled, allows using `$TURBO_EXTENDS$` in array fields.
    /// This will change the default behavior of overriding the field to instead
    /// append.
    pub turbo_extends_keyword: bool,
    /// Enable extending from a non-root `turbo.json`
    ///
    /// When enabled, allows using extends targeting `turbo.json`s other than
    /// root. All `turbo.json` must still extend from the root `turbo.json`
    /// first.
    pub non_root_extends: bool,
}

impl FutureFlags {
    /// Create a new FutureFlags with all features disabled
    pub fn new() -> Self {
        Self::default()
    }
}
