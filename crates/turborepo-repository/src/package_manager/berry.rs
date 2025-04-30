use tracing::debug;
use turbopath::AbsoluteSystemPath;

use super::yarnrc;

pub fn link_workspace_packages(repo_root: &AbsoluteSystemPath) -> bool {
    let yarnrc_config = yarnrc::YarnRc::from_file(repo_root)
        .inspect_err(|e| debug!("unable to read yarnrc: {e}"))
        .unwrap_or_default();
    yarnrc_config.enable_transparent_workspaces
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;

    #[test_case(None, true)]
    #[test_case(Some(false), false)]
    #[test_case(Some(true), true)]
    fn test_link_workspace_packages(enabled: Option<bool>, expected: bool) {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        if let Some(enabled) = enabled {
            repo_root
                .join_component(yarnrc::YARNRC_FILENAME)
                .create_with_contents(format!("enableTransparentWorkspaces: {enabled}"))
                .unwrap();
        }
        let actual = link_workspace_packages(repo_root);
        assert_eq!(actual, expected);
    }
}
