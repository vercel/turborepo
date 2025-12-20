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
///
/// Note: Currently all previous future flags (turboExtendsKeyword,
/// nonRootExtends) have been graduated and are now enabled by default.
#[derive(Serialize, Default, Debug, Copy, Clone, Iterable, Deserializable, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[deserializable()]
pub struct FutureFlags {}

impl FutureFlags {
    /// Create a new FutureFlags
    pub fn new() -> Self {
        Self::default()
    }
}
