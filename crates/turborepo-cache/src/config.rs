use std::str::FromStr;

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

use crate::{CacheActions, CacheConfig};

#[derive(Debug, Error, Diagnostic, PartialEq)]
pub enum Error {
    #[error("keys cannot be duplicated, found `{key}` multiple times")]
    DuplicateKeys {
        #[source_code]
        text: String,
        key: &'static str,
        #[label]
        span: Option<SourceSpan>,
    },
    #[error("actions cannot be duplicated, found `{action}` multiple times")]
    DuplicateActions {
        #[source_code]
        text: String,
        action: &'static str,
        #[label]
        span: Option<SourceSpan>,
    },
    #[error("invalid cache type and action pair, found `{pair}`, expected colon separated pair")]
    InvalidCacheTypeAndAction {
        #[source_code]
        text: String,
        pair: String,
        #[label]
        span: Option<SourceSpan>,
    },
    #[error("invalid cache action `{c}`")]
    InvalidCacheAction {
        #[source_code]
        text: String,
        c: char,
        #[label]
        span: Option<SourceSpan>,
    },
    #[error("invalid cache type `{s}`, expected `local` or `remote`")]
    InvalidCacheType {
        #[source_code]
        text: String,
        s: String,
        #[label]
        span: Option<SourceSpan>,
    },
}

impl Error {
    pub fn add_text(mut self, new_text: impl Into<String>) -> Self {
        match &mut self {
            Self::DuplicateKeys { text, .. } => *text = new_text.into(),
            Self::DuplicateActions { text, .. } => *text = new_text.into(),
            Self::InvalidCacheTypeAndAction { text, .. } => *text = new_text.into(),
            Self::InvalidCacheAction { text, .. } => *text = new_text.into(),
            Self::InvalidCacheType { text, .. } => *text = new_text.into(),
        }

        self
    }

    pub fn add_span(mut self, new_span: SourceSpan) -> Self {
        match &mut self {
            Self::DuplicateKeys { span, .. } => *span = Some(new_span),
            Self::DuplicateActions { span, .. } => *span = Some(new_span),
            Self::InvalidCacheTypeAndAction { span, .. } => *span = Some(new_span),
            Self::InvalidCacheAction { span, .. } => *span = Some(new_span),
            Self::InvalidCacheType { span, .. } => *span = Some(new_span),
        }

        self
    }
}

impl FromStr for CacheConfig {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut cache = CacheConfig {
            local: CacheActions {
                read: false,
                write: false,
            },
            remote: CacheActions {
                read: false,
                write: false,
            },
        };

        if s.is_empty() {
            return Ok(cache);
        }

        let mut seen_local = false;
        let mut seen_remote = false;
        let mut idx = 0;

        for action in s.split(',') {
            let (key, value) = action
                .split_once(':')
                .ok_or(Error::InvalidCacheTypeAndAction {
                    text: s.to_string(),
                    pair: action.to_string(),
                    span: Some(SourceSpan::new(idx.into(), action.len().into())),
                })?;

            match key {
                "local" => {
                    if seen_local {
                        return Err(Error::DuplicateKeys {
                            text: s.to_string(),
                            key: "local",
                            span: Some(SourceSpan::new(idx.into(), key.len().into())),
                        });
                    }

                    seen_local = true;
                    cache.local = CacheActions::from_str(value).map_err(|err| {
                        err.add_text(s).add_span(SourceSpan::new(
                            (idx + key.len() + 1).into(),
                            key.len().into(),
                        ))
                    })?;
                }
                "remote" => {
                    if seen_remote {
                        return Err(Error::DuplicateKeys {
                            text: s.to_string(),
                            key: "remote",
                            span: Some(SourceSpan::new(idx.into(), key.len().into())),
                        });
                    }

                    seen_remote = true;
                    cache.remote = CacheActions::from_str(value).map_err(|err| {
                        err.add_text(s).add_span(SourceSpan::new(
                            (idx + key.len() + 1).into(),
                            value.len().into(),
                        ))
                    })?
                }
                ty => {
                    return Err(Error::InvalidCacheType {
                        text: s.to_string(),
                        s: ty.to_string(),
                        span: Some(SourceSpan::new(idx.into(), ty.len().into())),
                    })
                }
            }

            idx += action.len() + 1;
        }
        Ok(cache)
    }
}

impl FromStr for CacheActions {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut cache = CacheActions {
            read: false,
            write: false,
        };

        for c in s.chars() {
            match c {
                'r' => {
                    if cache.read {
                        return Err(Error::DuplicateActions {
                            text: s.to_string(),
                            action: "r (read)",
                            span: None,
                        });
                    }
                    cache.read = true;
                }

                'w' => {
                    if cache.write {
                        return Err(Error::DuplicateActions {
                            text: s.to_string(),
                            action: "w (write)",
                            span: None,
                        });
                    }
                    cache.write = true;
                }
                _ => {
                    return Err(Error::InvalidCacheAction {
                        c,
                        text: String::new(),
                        span: None,
                    })
                }
            }
        }

        Ok(cache)
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;

    #[test_case("local:r,remote:w", Ok(CacheConfig { local: CacheActions { read: true, write: false }, remote: CacheActions { read: false, write: true } }) ; "local:r,remote:w"
    )]
    #[test_case("local:r", Ok(CacheConfig { local: CacheActions { read: true, write: false }, remote: CacheActions { read: false, write: false } }) ; "local:r"
    )]
    #[test_case("local:", Ok(CacheConfig { local: CacheActions { read: false, write: false }, remote: CacheActions { read: false, write: false } }) ; "empty action"
    )]
    #[test_case("local:,remote:", Ok(CacheConfig { local: CacheActions { read: false, write: false }, remote: CacheActions { read: false, write: false } }) ; "multiple empty actions"
    )]
    #[test_case("local:,remote:r", Ok(CacheConfig { local: CacheActions { read: false, write: false }, remote: CacheActions { read: true, write: false } }) ; "local: empty, remote:r"
    )]
    #[test_case("", Ok(CacheConfig { local: CacheActions { read: false, write: false }, remote: CacheActions { read: false, write: false } }) ; "empty"
    )]
    #[test_case("local:r,local:w", Err(Error::DuplicateKeys { text: "local:r,local:w".to_string(), key: "local", span: Some(SourceSpan::new(8.into(), 5.into())) }) ; "duplicate local key"
    )]
    #[test_case("local:rr", Err(Error::DuplicateActions { text: "local:rr".to_string(), action: "r (read)", span: Some(SourceSpan::new(6.into(), 5.into())) }) ; "duplicate action")]
    #[test_case("remote:r,local=rx", Err(Error::InvalidCacheTypeAndAction { text: "remote:r,local=rx".to_string(), pair: "local=rx".to_string(), span: Some(SourceSpan::new(9.into(), 8.into())) }) ; "invalid key action pair")]
    #[test_case("local:rx", Err(Error::InvalidCacheAction { c: 'x', text: "local:rx".to_string(), span: Some(SourceSpan::new(6.into(), 5.into())) }) ; "invalid action")]
    #[test_case("file:r", Err(Error::InvalidCacheType { s: "file".to_string(), text: "file:r".to_string(), span: Some(SourceSpan::new(0.into(), 4.into())) }) ; "invalid cache type")]
    fn test_cache_config(s: &str, expected: Result<CacheConfig, Error>) {
        assert_eq!(CacheConfig::from_str(s), expected);
    }
}
