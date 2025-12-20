use turbopath::AbsoluteSystemPath;
use turborepo_api_client::CacheClient;
use turborepo_auth::Token;

use super::{write_token, Error};
use crate::{commands::CommandBase, opts::APIClientOpts, rewrite_json};

#[derive(Default, Debug, PartialEq)]
struct ManualLoginOptions<'a> {
    api_url: Option<&'a str>,
    team_identifier: Option<TeamIdentifier<&'a str>>,
    token: Option<&'a str>,
}

#[derive(Debug, PartialEq)]
struct ResolvedManualLoginOptions {
    api_url: String,
    team_identifier: TeamIdentifier<String>,
    token: String,
}

#[derive(Debug, PartialEq)]
enum TeamIdentifier<T> {
    Id(T),
    Slug(T),
}

/// Manually write a turborepo token, API url, teamid
pub async fn login_manual(base: &mut CommandBase, force: bool) -> Result<(), Error> {
    let manual_login_opts = force
        .then(ManualLoginOptions::default)
        .unwrap_or_else(|| ManualLoginOptions::from(&base.opts().api_client_opts));
    let mut api_client = base.api_client()?;
    // fill in the missing information via prompts
    let ResolvedManualLoginOptions {
        api_url,
        team_identifier,
        token,
    } = manual_login_opts.resolve()?;
    // Check credentials
    api_client.with_base_url(api_url);
    let token = Token::new(token);
    check_credentials(&api_client, &token, &team_identifier).await?;
    // update global config with token
    write_token(base, token)?;
    // ensure api url & team id/slug are present in turbo.json
    let turbo_json_path = base.root_turbo_json_path()?;
    write_remote(&turbo_json_path, api_client.base_url(), team_identifier)?;
    Ok(())
}

impl<'a> From<&'a APIClientOpts> for ManualLoginOptions<'a> {
    fn from(value: &'a APIClientOpts) -> Self {
        let api_url = Some(value.api_url.as_str())
            // We ignore the default value for api_url
            .filter(|api_url| *api_url != crate::config::ConfigurationOptions::default().api_url());
        let team_id = value.team_id.as_deref().map(TeamIdentifier::Id);
        let team_slug = value.team_slug.as_deref().map(TeamIdentifier::Slug);
        let team_identifier = team_id.or(team_slug);
        // We always ask for a token even if one is present as the user probably wants a
        // new token when they run `turbo login`.
        let token = None;
        ManualLoginOptions {
            api_url,
            team_identifier,
            token,
        }
    }
}

impl TeamIdentifier<String> {
    fn as_tuple(&self) -> (Option<&str>, Option<&str>) {
        match self {
            TeamIdentifier::Id(id) => (Some(id.as_str()), None),
            TeamIdentifier::Slug(slug) => (None, Some(slug.as_str())),
        }
    }
}

impl ManualLoginOptions<'_> {
    fn resolve(&self) -> Result<ResolvedManualLoginOptions, Error> {
        let Self {
            api_url,
            team_identifier,
            token,
        } = self;

        let api_url = match api_url {
            Some(api_url) => api_url.to_string(),
            None => Self::ask("Remote Cache URL", false)?,
        };

        let team_identifier = match team_identifier {
            Some(TeamIdentifier::Id(id)) => TeamIdentifier::Id(id.to_string()),
            Some(TeamIdentifier::Slug(slug)) => TeamIdentifier::Slug(slug.to_string()),
            None => {
                // figure out
                let ask_for_team_id = dialoguer::Select::new()
                    .with_prompt("How do you want to specify your team?")
                    .items(&["id", "slug"])
                    .default(0)
                    .interact()?
                    == 0;
                if ask_for_team_id {
                    TeamIdentifier::Id(Self::ask("Team Id", false)?)
                } else {
                    TeamIdentifier::Slug(Self::ask("Team slug", false)?)
                }
            }
        };

        let token = match token {
            Some(token) => token.to_string(),
            None => Self::ask("Enter token", true)?,
        };
        Ok(ResolvedManualLoginOptions {
            api_url,
            team_identifier,
            token,
        })
    }

    fn ask(prompt: &'static str, pass: bool) -> Result<String, Error> {
        Ok(if pass {
            dialoguer::Password::new().with_prompt(prompt).interact()
        } else {
            dialoguer::Input::new().with_prompt(prompt).interact_text()
        }?)
    }
}

async fn check_credentials<T: CacheClient>(
    client: &T,
    token: &Token,
    team: &TeamIdentifier<String>,
) -> Result<(), Error> {
    let (team_id, team_slug) = team.as_tuple();
    let has_cache_access = token.has_cache_access(client, team_id, team_slug).await?;
    if has_cache_access {
        Ok(())
    } else {
        Err(Error::NoCacheAccess)
    }
}

fn write_remote(
    root_turbo_json: &AbsoluteSystemPath,
    api_url: &str,
    team_id: TeamIdentifier<String>,
) -> Result<(), Error> {
    let turbo_json_before = root_turbo_json
        .read_existing_to_string()?
        .unwrap_or_else(|| r#"{}"#.to_string());
    let with_api_url = rewrite_json::set_path(
        &turbo_json_before,
        &["remoteCache", "apiUrl"],
        &serde_json::to_string(api_url).unwrap(),
    )?;
    let (key, value) = match team_id {
        TeamIdentifier::Id(id) => ("teamId", id),
        TeamIdentifier::Slug(slug) => ("teamSlug", slug),
    };
    let with_team = rewrite_json::set_path(
        &with_api_url,
        &["remoteCache", key],
        &serde_json::to_string(&value).unwrap(),
    )?;
    root_turbo_json.ensure_dir()?;
    root_turbo_json.create_with_contents(with_team)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_default_api_url_filtered_out() {
        let api_opts = APIClientOpts {
            api_url: "https://vercel.com/api".into(),
            team_id: None,
            team_slug: None,
            token: None,
            timeout: 0,
            upload_timeout: 0,
            login_url: "".into(),
            preflight: false,
            sso_login_callback_port: None,
        };
        let login_opts = ManualLoginOptions::from(&api_opts);
        assert_eq!(
            login_opts,
            ManualLoginOptions {
                api_url: None,
                team_identifier: None,
                token: None
            }
        );
    }

    #[test]
    fn test_finds_existing_values() {
        let api_opts = APIClientOpts {
            api_url: "https://my-remote-cache.com".into(),
            team_slug: Some("custom-cache".into()),
            team_id: None,
            token: Some("token".into()),
            timeout: 0,
            upload_timeout: 0,
            login_url: "".into(),
            preflight: false,
            sso_login_callback_port: None,
        };
        let login_opts = ManualLoginOptions::from(&api_opts);
        assert_eq!(
            login_opts,
            ManualLoginOptions {
                api_url: Some("https://my-remote-cache.com"),
                team_identifier: Some(TeamIdentifier::Slug("custom-cache")),
                token: None
            }
        );
    }

    #[test]
    fn test_write_remote_handles_missing_file() {
        let tmpdir = tempdir().unwrap();
        let tmpdir_path = AbsoluteSystemPath::new(tmpdir.path().to_str().unwrap()).unwrap();
        let root_turbo_json = tmpdir_path.join_component("turbo.json");
        write_remote(
            &root_turbo_json,
            "http://example.com",
            TeamIdentifier::Slug("slugworth".into()),
        )
        .unwrap();
        let contents = root_turbo_json.read_existing_to_string().unwrap().unwrap();
        assert_snapshot!(contents, @r#"{"remoteCache":{"teamSlug":"slugworth","apiUrl":"http://example.com"}}"#);
    }

    #[test]
    fn test_keeps_existing_remote_cache() {
        let tmpdir = tempdir().unwrap();
        let tmpdir_path = AbsoluteSystemPath::new(tmpdir.path().to_str().unwrap()).unwrap();
        let root_turbo_json = tmpdir_path.join_component("turbo.json");
        root_turbo_json
            .create_with_contents(r#"{"remoteCache": {"enabled": true}}"#)
            .unwrap();
        write_remote(
            &root_turbo_json,
            "http://example.com",
            TeamIdentifier::Slug("slugworth".into()),
        )
        .unwrap();
        let contents = root_turbo_json.read_existing_to_string().unwrap().unwrap();
        assert_snapshot!(contents, @r#"{"remoteCache": {"teamSlug":"slugworth","apiUrl":"http://example.com","enabled": true}}"#);
    }
}
