//! turbo.json configuration re-exports.

pub use turborepo_microfrontends_config::{TurboJsonReader, UnifiedTurboJsonLoader};
pub use turborepo_turbo_json::{FutureFlags, RawRootTurboJson, RawTurboJson, TurboJson};

pub mod parser {
    pub use turborepo_turbo_json::parser::BiomeParseError as Error;
}
