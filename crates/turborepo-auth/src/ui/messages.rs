use turborepo_ui::{BOLD, CYAN, ColorConfig};

/// How a completed login tells the user to connect this repository's Remote
/// Cache.
///
/// `turbo link` reads teams from Vercel's account API, so it is only a next
/// step for a repository that logs in to Vercel without a team already
/// configured. Every other case is told what actually connects it.
#[derive(Debug, Clone, Copy)]
pub enum RemoteCacheHint {
    /// Nothing is connected yet, and `turbo link` can pick a team.
    RunLink,
    /// A team is already configured, so there is nothing left to link.
    AlreadyConnected,
    /// A self-hosted Remote Cache is configured in `turbo.json`, and its API
    /// isn't required to implement the routes `turbo link` calls.
    ConfigureInTurboJson,
}

impl RemoteCacheHint {
    pub fn for_login(is_vercel: bool, team_id: Option<&str>, team_slug: Option<&str>) -> Self {
        match (is_vercel, team_id.or(team_slug).is_some()) {
            (_, true) => Self::AlreadyConnected,
            (true, false) => Self::RunLink,
            (false, false) => Self::ConfigureInTurboJson,
        }
    }

    fn lines(self) -> (&'static str, Option<&'static str>) {
        match self {
            Self::RunLink => (
                "To connect to your Remote Cache, run the following in any turborepo:",
                Some("  npx turbo link"),
            ),
            Self::AlreadyConnected => (
                "This repository is already connected to a Remote Cache.",
                None,
            ),
            Self::ConfigureInTurboJson => (
                "To connect your self-hosted Remote Cache, set `remoteCache.apiUrl` and \
                 `remoteCache.teamSlug` in turbo.json:",
                Some("  https://turborepo.dev/docs/core-concepts/remote-caching#self-hosting"),
            ),
        }
    }
}

pub fn print_cli_authorized(user: &str, color_config: &ColorConfig, hint: RemoteCacheHint) {
    print!("{}", authorized_message(user, color_config, hint));
}

fn authorized_message(user: &str, color_config: &ColorConfig, hint: RemoteCacheHint) -> String {
    let (summary, command) = hint.lines();
    let mut message = format!(
        "
{} Turborepo CLI authorized for {}
{}
",
        color_config.rainbow(">>> Success!"),
        user,
        color_config.apply(CYAN.apply_to(summary)),
    );

    if let Some(command) = command {
        message.push_str(&format!("{}\n", color_config.apply(BOLD.apply_to(command))));
    }

    message
}

#[cfg(test)]
mod tests {
    use turborepo_ui::ColorConfig;

    use super::*;

    fn message(hint: RemoteCacheHint) -> String {
        authorized_message("user@example.com", &ColorConfig::new(false), hint)
    }

    #[test]
    fn self_hosted_login_points_at_turbo_json_instead_of_linking() {
        let message = message(RemoteCacheHint::for_login(false, None, None));

        assert!(!message.contains("turbo link"), "{message}");
        assert!(message.contains("remoteCache.teamSlug"), "{message}");
    }

    #[test]
    fn login_into_a_connected_repository_does_not_ask_to_link_again() {
        let connected = [
            (false, None, Some("my-team")),
            (false, Some("team_123"), None),
            (true, None, Some("my-team")),
            (true, Some("team_123"), None),
        ];

        for (is_vercel, team_id, team_slug) in connected {
            let message = message(RemoteCacheHint::for_login(is_vercel, team_id, team_slug));

            assert!(!message.contains("turbo link"), "{message}");
            assert!(message.contains("already connected"), "{message}");
        }
    }

    #[test]
    fn unlinked_vercel_login_points_at_turbo_link() {
        let message = message(RemoteCacheHint::for_login(true, None, None));

        assert!(message.contains("npx turbo link"), "{message}");
        assert!(!message.contains("already connected"), "{message}");
    }
}
