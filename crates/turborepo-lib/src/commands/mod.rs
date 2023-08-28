use std::cell::OnceCell;

use anyhow::{anyhow, Error, Result};
use dirs_next::config_dir;
use sha2::{Digest, Sha256};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_api_client::APIClient;
use turborepo_ui::UI;

use crate::{
    config::{ConfigurationOptions, TurborepoConfigBuilder},
    Args,
};

pub(crate) mod bin;
pub(crate) mod daemon;
pub(crate) mod generate;
pub(crate) mod info;
pub(crate) mod link;
pub(crate) mod login;
pub(crate) mod logout;
pub(crate) mod prune;
pub(crate) mod run;
pub(crate) mod unlink;

#[derive(Debug)]
pub struct CommandBase {
    pub repo_root: AbsoluteSystemPathBuf,
    pub ui: UI,
    pub global_config_path: Option<AbsoluteSystemPathBuf>,
    pub config: OnceCell<ConfigurationOptions>,
    args: Args,
    version: &'static str,
}

impl CommandBase {
    pub fn new(
        args: Args,
        repo_root: AbsoluteSystemPathBuf,
        version: &'static str,
        ui: UI,
    ) -> Result<Self> {
        Ok(Self {
            repo_root,
            ui,
            args,
            global_config_path: None,
            config: OnceCell::new(),
            version,
        })
    }

    #[cfg(test)]
    pub fn with_global_config_path(mut self, path: AbsoluteSystemPathBuf) -> Self {
        self.global_config_path = Some(path);
        self
    }

    fn config_init(&self) -> Result<ConfigurationOptions, anyhow::Error> {
        TurborepoConfigBuilder::new(self)
            // The below should be deprecated and removed.
            .with_api_url(self.args.api.clone())
            .with_login_url(self.args.login.clone())
            .with_team_slug(self.args.team.clone())
            .with_token(self.args.token.clone())
            .with_timeout(self.args.remote_cache_timeout)
            .build()
    }

    pub fn config(&self) -> Result<&ConfigurationOptions, anyhow::Error> {
        self.config.get_or_try_init(|| self.config_init())
    }

    // Getting all of the paths.
    fn global_config_path(&self) -> Result<AbsoluteSystemPathBuf, Error> {
        if let Some(global_config_path) = self.global_config_path.clone() {
            return Ok(global_config_path);
        }
        Ok(AbsoluteSystemPathBuf::try_from(
            config_dir()
                .map(|p| p.join("turborepo").join("config.json"))
                .ok_or(anyhow!("No global config path"))?,
        )?)
    }
    fn local_config_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_components(&[".turbo", "config.json"])
    }
    fn root_package_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("package.json")
    }
    fn root_turbo_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("turbo.json")
    }

    pub fn args(&self) -> &Args {
        &self.args
    }

    pub fn api_client(&self) -> Result<APIClient> {
        let config = self.config()?;
        let args = self.args();

        let api_url = config.api_url();
        let timeout = config.timeout();
        Ok(APIClient::new(
            api_url,
            timeout,
            self.version,
            args.preflight,
        )?)
    }

    pub fn daemon_file_root(&self) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::new(std::env::temp_dir().to_str().expect("UTF-8 path"))
            .expect("temp dir is valid")
            .join_component("turbod")
            .join_component(self.repo_hash().as_str())
    }

    fn repo_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.repo_root.as_bytes());
        hex::encode(&hasher.finalize()[..8])
    }

    /// Current working directory for the turbo command
    pub fn cwd(&self) -> &AbsoluteSystemPath {
        // Earlier in execution
        // self.cli_args.cwd = Some(repo_root.as_path())
        // happens.
        // We directly use repo_root to avoid converting back to absolute system path
        &self.repo_root
    }

    pub fn version(&self) -> &'static str {
        self.version
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_ui::UI;

    use crate::get_version;

    #[cfg(not(target_os = "windows"))]
    #[test_case("/tmp/turborepo", "6e0cfa616f75a61c"; "basic example")]
    fn test_repo_hash(path: &str, expected_hash: &str) {
        use super::CommandBase;
        use crate::Args;

        let args = Args::default();
        let repo_root = AbsoluteSystemPathBuf::new(path).unwrap();
        let command_base = CommandBase::new(args, repo_root, get_version(), UI::new(true)).unwrap();

        let hash = command_base.repo_hash();

        assert_eq!(hash, expected_hash);
        assert_eq!(hash.len(), 16);
    }

    #[cfg(target_os = "windows")]
    #[test_case("C:\\\\tmp\\turborepo", "0103736e6883e35f"; "basic example")]
    fn test_repo_hash_win(path: &str, expected_hash: &str) {
        use super::CommandBase;
        use crate::Args;

        let args = Args::default();
        let repo_root = AbsoluteSystemPathBuf::new(path).unwrap();
        let command_base = CommandBase::new(args, repo_root, get_version(), UI::new(true)).unwrap();

        let hash = command_base.repo_hash();

        assert_eq!(hash, expected_hash);
        assert_eq!(hash.len(), 16);
    }
}
