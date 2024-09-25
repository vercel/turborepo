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
    #[error("invalid cache type `{s}`, expected `fs` or `remote`")]
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
            fs: CacheActions {
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

        let mut seen_fs = false;
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
                "fs" => {
                    if seen_fs {
                        return Err(Error::DuplicateKeys {
                            text: s.to_string(),
                            key: "fs",
                            span: Some(SourceSpan::new(idx.into(), key.len().into())),
                        });
                    }

                    seen_fs = true;
                    cache.fs = CacheActions::from_str(value).map_err(|err| {
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

    #[test_case("fs:r,remote:w", Ok(CacheConfig { fs: CacheActions { read: true, write: false }, remote: CacheActions { read: false, write: true } }) ; "fs:r,remote:w"
    )]
    #[test_case("fs:r", Ok(CacheConfig { fs: CacheActions { read: true, write: false }, remote: CacheActions { read: false, write: false } }) ; "fs:r"
    )]
    #[test_case("fs:", Ok(CacheConfig { fs: CacheActions { read: false, write: false }, remote: CacheActions { read: false, write: false } }) ; "empty action"
    )]
    #[test_case("fs:,remote:", Ok(CacheConfig { fs: CacheActions { read: false, write: false }, remote: CacheActions { read: false, write: false } }) ; "multiple empty actions"
    )]
    #[test_case("fs:,remote:r", Ok(CacheConfig { fs: CacheActions { read: false, write: false }, remote: CacheActions { read: true, write: false } }) ; "fs: empty, remote:r"
    )]
    #[test_case("", Ok(CacheConfig { fs: CacheActions { read: false, write: false }, remote: CacheActions { read: false, write: false } }) ; "empty"
    )]
    #[test_case("fs:r,fs:w", Err(Error::DuplicateKeys { text: "fs:r,fs:w".to_string(), key: "fs", span: Some(SourceSpan::new(5.into(), 2.into())) }) ; "duplicate fs key"
    )]
    #[test_case("fs:rr", Err(Error::DuplicateActions { text: "fs:rr".to_string(), action: "r (read)", span: Some(SourceSpan::new(3.into(), 2.into())) }) ; "duplicate action")]
    #[test_case("remote:r,fs=rx", Err(Error::InvalidCacheTypeAndAction { text: "remote:r,fs=rx".to_string(), pair: "fs=rx".to_string(), span: Some(SourceSpan::new(9.into(), 5.into())) }) ; "invalid key action pair")]
    #[test_case("fs:rx", Err(Error::InvalidCacheAction { c: 'x', text: "fs:rx".to_string(), span: Some(SourceSpan::new(3.into(), 2.into())) }) ; "invalid action")]
    #[test_case("file:r", Err(Error::InvalidCacheType { s: "file".to_string(), text: "file:r".to_string(), span: Some(SourceSpan::new(0.into(), 4.into())) }) ; "invalid cache type")]
    fn test_cache_config(s: &str, expected: Result<CacheConfig, Error>) {
        assert_eq!(CacheConfig::from_str(s), expected);
    }
}
