use std::path::PathBuf;

use anyhow::Result;
use tokio::sync::OnceCell;

use crate::{
    client::APIClient,
    config::{
        default_user_config_path, get_repo_config_path, RepoConfig, RepoConfigLoader, UserConfig,
        UserConfigLoader,
    },
    ui::UI,
    Args,
};

pub(crate) mod bin;
pub(crate) mod link;
pub(crate) mod login;
pub(crate) mod logout;

pub struct CommandBase {
    pub repo_root: PathBuf,
    pub ui: UI,
    user_config: OnceCell<UserConfig>,
    repo_config: OnceCell<RepoConfig>,
    args: Args,
}

impl CommandBase {
    pub fn new(args: Args, repo_root: PathBuf) -> Result<Self> {
        Ok(Self {
            repo_root,
            ui: args.ui(),
            args,
            repo_config: OnceCell::new(),
            user_config: OnceCell::new(),
        })
    }

    fn create_repo_config(&self) -> Result<()> {
        let repo_config_path = get_repo_config_path(&self.repo_root);

        let repo_config = RepoConfigLoader::new(repo_config_path)
            .with_api(self.args.api.clone())
            .with_login(self.args.login.clone())
            .with_team_slug(self.args.team.clone())
            .load()?;

        self.repo_config.set(repo_config)?;

        Ok(())
    }

    fn create_user_config(&self) -> Result<()> {
        let user_config = UserConfigLoader::new(default_user_config_path()?)
            .with_token(self.args.token.clone())
            .load()?;
        self.user_config.set(user_config)?;

        Ok(())
    }

    pub fn repo_config_mut(&mut self) -> Result<&mut RepoConfig> {
        if self.repo_config.get().is_none() {
            self.create_repo_config()?;
        }

        Ok(self.repo_config.get_mut().unwrap())
    }

    pub fn repo_config(&self) -> Result<&RepoConfig> {
        if self.repo_config.get().is_none() {
            self.create_repo_config()?;
        }

        Ok(self.repo_config.get().unwrap())
    }

    pub fn user_config_mut(&mut self) -> Result<&mut UserConfig> {
        if self.user_config.get().is_none() {
            self.create_user_config()?;
        }

        Ok(self.user_config.get_mut().unwrap())
    }

    pub fn user_config(&self) -> Result<&UserConfig> {
        if self.user_config.get().is_none() {
            self.create_user_config()?;
        }

        Ok(self.user_config.get().unwrap())
    }

    pub fn api_client(&mut self) -> Result<Option<APIClient>> {
        let repo_config = self.repo_config()?;
        let api_url = repo_config.api_url();
        let user_config = self.user_config()?;
        if let Some(token) = user_config.token() {
            Ok(Some(APIClient::new(token, api_url)?))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod test {
    use std::{fs, net::SocketAddr};

    use anyhow::Result;
    use axum::{routing::get, Json, Router};
    use tempfile::NamedTempFile;
    use tokio::sync::OnceCell;

    use crate::{
        client::{Membership, Role, Team, TeamsResponse, User, UserResponse},
        commands::{link, CommandBase},
        config::{RepoConfigLoader, UserConfigLoader},
        ui::UI,
        Args,
    };

    #[tokio::test]
    async fn test_link() {
        let user_config_file = NamedTempFile::new().unwrap();
        fs::write(user_config_file.path(), r#"{ "token": "hello" }"#).unwrap();
        let repo_config_file = NamedTempFile::new().unwrap();
        fs::write(
            repo_config_file.path(),
            r#"{ "apiurl": "http://localhost:3000" }"#,
        )
        .unwrap();

        tokio::spawn(start_test_server());
        let base = CommandBase {
            repo_root: Default::default(),
            ui: UI::new(false),
            user_config: OnceCell::from(
                UserConfigLoader::new(user_config_file.path().to_path_buf())
                    .with_token(Some("token".to_string()))
                    .load()
                    .unwrap(),
            ),
            repo_config: OnceCell::from(
                RepoConfigLoader::new(repo_config_file.path().to_path_buf())
                    .with_api(Some("http://localhost:3000".to_string()))
                    .with_login(Some("http://localhost:3000".to_string()))
                    .load()
                    .unwrap(),
            ),
            args: Args::default(),
        };

        link::link(base, false).await.unwrap();
    }

    async fn start_test_server() -> Result<()> {
        let app = Router::new()
            // `GET /` goes to `root`
            .route(
                "/v2/teams",
                get(|| async move {
                    Json(TeamsResponse {
                        teams: vec![Team {
                            id: "vercel".to_string(),
                            slug: "vercel".to_string(),
                            name: "vercel".to_string(),
                            created_at: 0,
                            created: Default::default(),
                            membership: Membership::new(Role::Owner),
                        }],
                    })
                }),
            )
            .route(
                "/v2/user",
                get(|| async move {
                    Json(UserResponse {
                        user: User {
                            id: "my_user_id".to_string(),
                            username: "my_username".to_string(),
                            email: "my_email".to_string(),
                            name: None,
                            created_at: 0,
                        },
                    })
                }),
            );
        let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

        Ok(axum_server::bind(addr)
            .serve(app.into_make_service())
            .await?)
    }
}
