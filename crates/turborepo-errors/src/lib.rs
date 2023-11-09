//! Turborepo's library for high quality errors

use std::{fmt::Display, ops::Deref, sync::Arc};

use serde::{Deserialize, Serialize};

pub trait Sourced {
    fn with_provenance(self, provenance: Option<Arc<Provenance>>) -> Self;

    fn provenance(&self) -> Option<Arc<Provenance>>;
}

#[derive(Clone, Debug)]
pub struct WithSource<T> {
    value: T,
    provenance: Option<Arc<Provenance>>,
}

impl<T> WithSource<T> {
    pub fn new(value: T, provenance: Option<Arc<Provenance>>) -> Self {
        Self { value, provenance }
    }
}

impl<T> Deref for WithSource<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> Sourced for WithSource<T> {
    fn with_provenance(self, provenance: Option<Arc<Provenance>>) -> Self {
        Self::new(self.value, provenance)
    }

    fn provenance(&self) -> Option<Arc<Provenance>> {
        self.provenance.clone()
    }
}

impl<T: Sourced, E: Sourced> Sourced for Result<T, E> {
    fn with_provenance(self, provenance: Option<Arc<Provenance>>) -> Self {
        match self {
            Ok(value) => Ok(value.with_provenance(provenance)),
            Err(err) => Err(err.with_provenance(provenance)),
        }
    }
    fn provenance(&self) -> Option<Arc<Provenance>> {
        match self {
            Ok(value) => value.provenance(),
            Err(err) => err.provenance(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provenance {
    // TODO: Add line/column numbers
    TurboJson,
    EnvironmentVariable { name: String },
    Flag { name: String },
}

impl Display for Provenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provenance::TurboJson => write!(f, "from turbo.json"),
            Provenance::EnvironmentVariable { name } => write!(f, "environment variable {}", name),
            Provenance::Flag { name } => {
                write!(f, "flag --{}", name)
            }
        }
    }
}

impl Provenance {
    pub fn from_flag(name: impl Into<String>) -> Option<Arc<Provenance>> {
        Some(Arc::new(Provenance::Flag { name: name.into() }))
    }
}
