//! The Next.js build output (`.next`) as the source of "which routes changed".
//!
//! Validation tooling: selection reads the source only. What two builds differ
//! in is the oracle that the routes a change reaches from source are checked
//! against.
//!
//! - [`manifest`]: the route entries of a build and the output files each one
//!   loads, read from the build's own manifests;
//! - [`fingerprint`]: per-route hashes of that output with everything that is
//!   not content normalized away ([`modules`] splits Turbopack chunks into
//!   modules with canonical identifiers, [`css`] keeps stylesheets as rule
//!   sets);
//! - [`invokes`]: the routes a route invokes at runtime, and the effective
//!   fingerprint (own output plus the output of what it invokes).

pub mod css;
pub mod diff;
pub mod fingerprint;
pub mod invokes;
pub mod manifest;
pub mod modules;
pub mod sites;
