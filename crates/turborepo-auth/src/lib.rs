#![feature(fs_try_exists)]
//! Turborepo's library for authenticating with the Vercel API.
//! Handles logging into Vercel, verifying SSO, and storing the token.

mod auth_file;
mod config_token;
mod error;
mod login;
mod login_server;
mod logout;
pub mod mocks;
mod sso;
mod sso_server;
mod ui;

use std::collections::HashMap;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

pub use self::{
    auth_file::*, config_token::ConfigToken, error::Error, login::*, login_server::*, logout::*,
    sso::*, sso_server::*,
};

pub const TURBOREPO_AUTH_FILE_NAME: &str = "auth.json";
pub const TURBOREPO_LEGACY_AUTH_FILE_NAME: &str = "config.json";
pub const TURBOREPO_CONFIG_DIR: &str = "turborepo";
pub const VERCEL_CONFIG_DIR: &str = "com.vercel.cli";
pub const VERCEL_AUTH_FILE_NAME: &str = "auth.json";

pub const DEFAULT_LOGIN_URL: &str = "https://vercel.com";
pub const DEFAULT_API_URL: &str = "https://vercel.com/api";

/// Source determines where the auth file should be read from. Each of the
/// variants has different initialization and permissions.
pub enum Source {
    /// A token passed in via the CLI. This is the most ephemeral of the auth
    /// sources. It will no-op on any and all fs write operations.
    CLI(String),
    /// Our custom auth file. This is allowed to read/write.
    // TODO(voz): At some point, we should deprecate having support for a token in the global
    // config so we can stop passing in the `String` here.
    Turborepo(AbsoluteSystemPathBuf, AbsoluteSystemPathBuf, String),
    /// The Vercel auth file. This is a read-only source issued from the Vercel
    /// CLI. Write operations will no-op.
    Vercel(AbsoluteSystemPathBuf),
}

/// Provider is an enum that contains either a token or a file. This is used
/// for holding a/many token(s), depending on the variant.
// TODO(voz)|team-input: This is my big "I need input". Some of this feels
// great, others not so much. It reall feels wrong to have methods like
// "insert" when the source is from the CLI, which we shouldn't insert or write
// to disk ever. However just being able to use a "Provider" for auth feels good
// for auth files. Looking for input here on how y'all feel about this DX.
pub enum Provider {
    Token(AuthToken),
    File(AuthFile),
}
impl Provider {
    /// Creates a new Auth enum from a Source.
    /// ## Arguments
    /// * `source`: The Source to create the Auth enum from.
    /// ## Returns
    /// * `Auth`: The Auth enum.
    /// ## Examples
    /// ```
    /// use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
    /// use turborepo_auth::{Provider, Source, DEFAULT_API_URL};
    ///
    /// let auth_file_path = AbsoluteSystemPath::new("/path/to/auth/file/auth.json").unwrap();
    /// let config_file_path = AbsoluteSystemPath::new("/path/to/config/file/config.json").unwrap();
    /// let vercel_file_path = AbsoluteSystemPath::new("/path/to/vercel/file/auth.json").unwrap();
    /// let api = DEFAULT_API_URL;
    ///
    /// // Create an Auth enum from a Vercel file.
    /// let vercel_auth_file: Provider = Provider::new(Source::Vercel(vercel_file_path));
    /// // Create an Auth enum from a file.
    /// let turbo_auth_file: Provider = Provider::new(Source::Turborepo(auth_file_path, config_file_path, api));
    /// // Create an Auth enum from a token.
    /// let auth_token: Provider = Provider::new(Source::CLI("test-token".to_string()));
    ///
    /// assert!(turbo_auth_file.is_file());
    /// assert!(vercel_auth_file.is_file());
    /// assert!(!auth_token.is_file()
    /// ```
    pub fn new(source: Source) -> Result<Self, Error> {
        match source {
            // Any token coming in from the CLI is a one-off, so we don't give it any file checks,
            // api keys, or permissions. If we add functionality, like refreshing tokens
            // or anything that might allow for a passed-in token to be written to disk,
            // we'll update this arm.
            Source::CLI(token) => Ok(Self::token_from_cli(token)),
            Source::Turborepo(auth_file_path, config_file_path, api) => {
                Self::file_from_turborepo_paths(auth_file_path, config_file_path, api)
            }
            Source::Vercel(vercel_file_path) => Self::file_from_vercel_path(vercel_file_path),
        }
    }
    fn token_from_cli(token: String) -> Self {
        Self::Token(AuthToken {
            token,
            api: "".to_string(),
        })
    }
    fn file_from_turborepo_paths(
        auth_file_path: AbsoluteSystemPathBuf,
        config_file_path: AbsoluteSystemPathBuf,
        api: String,
    ) -> Result<Self, Error> {
        if let Ok(auth_file) = read_auth_file(&auth_file_path) {
            return Ok(Self::File(auth_file));
        }
        // Check the config file for a token and convert it to the auth file if
        // possible.
        if let Ok(config_content) = config_file_path.read_to_string() {
            let config_token: ConfigToken = serde_json::from_str(&config_content)
                .map_err(|e| Error::FailedToDeserializeConfigToken { source: e })?;
            let auth_token: AuthToken = AuthToken {
                token: config_token.token,
                api: api.to_owned(),
            };
            // Create the new auth format and write it to disk right away so we have it in
            // the future.
            let mut auth_file = AuthFile::new(auth_file_path.to_owned());
            auth_file.insert(api.to_owned(), auth_token.token);
            auth_file.write_to_disk(&auth_file_path)?;
            return Ok(Self::File(auth_file));
        }
        Err(Error::FailedToFindAuthOrConfigFile {
            auth_path: auth_file_path,
            config_path: config_file_path,
        })
    }
    fn file_from_vercel_path(vercel_file_path: AbsoluteSystemPathBuf) -> Result<Self, Error> {
        let auth_file = read_auth_file(&vercel_file_path)?;
        Ok(Self::File(auth_file))
    }
    /// Attempts to create a Provider from a list of Sources in the order they
    /// are passed in through the Vec.
    ///
    /// The first one to succeed will be returned. If none succeed, a
    /// FailedToReadAuthFile error will be returned.
    pub fn try_sources(sources: Vec<Source>) -> Result<Self, Error> {
        for source in sources {
            match Self::new(source) {
                Ok(provider) => return Ok(provider),
                Err(e) => eprintln!("Error creating provider: {:?}", e),
            }
        }
        Err(Error::FailedToReadAuthFile {
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No auth file found in any of the provided sources",
            ),
            path: AbsoluteSystemPathBuf::from_cwd(TURBOREPO_AUTH_FILE_NAME).unwrap(),
        })
    }
    /// Determines if this enum is a file or a token.
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File(_))
    }
    /// Returns the underlying token. If the enum is a `Token`, it will return
    /// the token used to construct it. Otherwise, the `api` argument is used to
    /// look up the token in the file, if it exists.
    pub fn get_token(&self, api: &str) -> Option<AuthToken> {
        match self {
            Self::Token(t) => Some(t.clone()),
            Self::File(f) => f.get_token(api),
        }
    }
    pub fn tokens(&self) -> Option<&HashMap<String, String>> {
        match self {
            Self::Token(_) => None,
            Self::File(f) => Some(f.tokens()),
        }
    }
    pub fn clear_tokens(&mut self) {
        match self {
            Self::Token(_) => {}
            Self::File(f) => f.tokens_mut().clear(),
        }
    }
    pub fn remove(&mut self, api: &str) {
        match self {
            Self::Token(_) => {}
            Self::File(f) => f.remove(api),
        }
    }
    /// Inserts a token into the underlying Provider. If the enum is a `Token`,
    /// it will no-op.
    pub fn insert(&mut self, api: String, token: String) -> Option<String> {
        match self {
            Self::Token(t) => {
                let old_token = t.token.clone();
                t.token = token;
                Some(old_token)
            }
            Self::File(f) => f.insert(api, token),
        }
    }
    /// Writes the contents of the underlying Provider to disk. If the enum is a
    /// `Token`, it will no-op.
    pub fn write_to_disk(&self, path: &AbsoluteSystemPath) -> Result<(), Error> {
        match self {
            Self::Token(_) => Ok(()),
            Self::File(f) => f.write_to_disk(path),
        }
    }
}

fn read_auth_file(path: &AbsoluteSystemPath) -> Result<AuthFile, Error> {
    let content = path
        .read_to_string()
        .map_err(|e| Error::FailedToReadAuthFile {
            source: e,
            path: path.to_owned(),
        })?;
    serde_json::from_str(&content).map_err(|e| Error::FailedToDeserializeAuthFile { source: e })
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

    use serde_json::json;
    use tempfile::TempDir;
    use test_case::test_case;

    use super::*;

    fn create_temp_file(dir: &TempDir, file_name: &str, content: &str) -> AbsoluteSystemPathBuf {
        let file_path = dir.path().join(file_name);
        let full_file_path = AbsoluteSystemPathBuf::new(file_path.to_str().unwrap())
            .expect("Failed to create file path");
        let mut file = File::create(&full_file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        full_file_path
    }

    #[test_case(Source::CLI("cli-token".to_string()), "cli-token" ; "cli token")]
    async fn test_new_cli_provider(source: Source, expected_token: &str) {
        // Tests
        let provider_result = Provider::new(source);
        assert!(provider_result.is_ok());

        let provider = provider_result.unwrap();
        assert_eq!(
            provider.get_token(""),
            Some(AuthToken {
                token: expected_token.to_string(),
                api: "".to_string()
            })
        );
    }

    #[test_case(json!({
        "token": "test-token"
    }) ; "legacy turborepo auth file")]
    #[test_case(json!({
        "tokens": {
            "test-api": "test-token"
        }
    }); "new turborepo auth file")]
    #[test_case(json!({
        "// Note": "This is a Vercel auth file. It should be ignored.",
        "// Docs": "https://vercel.com/docs/cli#configuration",
        "token": "vercel-token"
    }))]
    async fn test_new_file_provider(file_content: serde_json::Value) {
        // Setup
        let tempdir = TempDir::new().expect("Failed to create temp dir");
        let auth_file_path = create_temp_file(
            &tempdir,
            TURBOREPO_AUTH_FILE_NAME,
            file_content.as_str().unwrap(),
        );
        let config_file_path = create_temp_file(
            &tempdir,
            TURBOREPO_LEGACY_AUTH_FILE_NAME,
            file_content.as_str().unwrap(),
        );
        let vercel_file_path = create_temp_file(
            &tempdir,
            format!("vercel/{}", VERCEL_AUTH_FILE_NAME).as_str(),
            file_content.as_str().unwrap(),
        );
        let api = DEFAULT_API_URL;

        let turborepo_source = Source::Turborepo(
            auth_file_path.clone(),
            config_file_path.clone(),
            api.to_string(),
        );
        let vercel_source = Source::Vercel(vercel_file_path.clone());

        // Tests
        // TODO(voz): These should do more. Basic for draft PR.
        let turbo_provider_result = Provider::new(turborepo_source);
        assert!(turbo_provider_result.is_ok());
        let vercel_provider_result = Provider::new(vercel_source);
        assert!(vercel_provider_result.is_ok());

        let turbo_provider = turbo_provider_result.unwrap();
        assert!(turbo_provider.is_file());
        let vercel_provider = vercel_provider_result.unwrap();
        assert!(vercel_provider.is_file());
    }

    #[test_case("cli-token" ; "cli token")]
    async fn test_provider_from_cli_source(token: &str) {
        let provider = Provider::new(Source::CLI(token.to_owned()))
            .expect("Failed to create provider from CLI source");
        assert_eq!(provider.get_token(""), Some(AuthToken::new(token, "")));
        assert!(matches!(provider, Provider::Token(_)));
    }
}
