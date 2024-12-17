use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPathBuf;

const TUI_PREFERENCES_PATH_COMPONENTS: &[&str] = &[".turbo", "preferences", "tui.json"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

pub struct PreferenceLoader {
    file_path: AbsoluteSystemPathBuf,
    config: Preferences,
}

impl PreferenceLoader {
    pub fn new(repo_root: &AbsoluteSystemPathBuf) -> Result<Self, Error> {
        let file_path = repo_root.join_components(TUI_PREFERENCES_PATH_COMPONENTS);
        let contents = file_path.read_existing_to_string()?;
        let config = contents
            .map(|string| serde_json::from_str(&string))
            .transpose()?
            .unwrap_or_default();

        Ok(Self { file_path, config })
    }

    pub fn is_task_list_visible(&self) -> bool {
        self.config.is_task_list_visible.unwrap_or(true)
    }

    pub fn set_is_task_list_visible(&mut self, value: Option<bool>) -> Result<(), Error> {
        self.config.is_task_list_visible = value;
        Ok(())
    }

    pub fn active_task(&self) -> Option<&str> {
        let active_task = self.config.active_task.as_deref()?;
        Some(active_task)
    }

    pub fn set_active_task(&mut self, value: Option<String>) -> Result<(), Error> {
        self.config.active_task = value;
        Ok(())
    }

    pub fn flush_to_disk(&self) -> Result<(), Error> {
        self.file_path.ensure_dir()?;
        self.file_path
            .create_with_contents(serde_json::to_string_pretty(&self.config)?)?;

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Preferences {
    pub is_task_list_visible: Option<bool>,
    pub active_task: Option<String>,
    pub is_pinned_task_selection: Option<bool>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            active_task: None,
            is_task_list_visible: Some(true),
            is_pinned_task_selection: Some(false),
        }
    }
}

#[cfg(test)]
mod test {
    use tempfile::tempdir;

    use super::*;

    fn create_loader(repo_root: AbsoluteSystemPathBuf) -> PreferenceLoader {
        PreferenceLoader::new(&repo_root).expect("Failed to create PreferenceLoader")
    }

    #[test]
    fn default_preferences() {
        let preferences = Preferences::default();
        assert_eq!(preferences.active_task, None);
        assert_eq!(preferences.is_task_list_visible, Some(true));
        assert_eq!(preferences.is_pinned_task_selection, Some(false));
    }

    #[test]
    fn task_list_visible_when_no_preferences() {
        let repo_root_tmp = tempdir().expect("Failed to create tempdir");
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");
        let loader = create_loader(repo_root);

        let visibility = PreferenceLoader::is_task_list_visible(&loader);
        assert!(visibility);
    }

    #[test]
    fn task_is_none_when_no_preferences() {
        let repo_root_tmp = tempdir().expect("Failed to create tempdir");
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let loader = create_loader(repo_root);

        let task = PreferenceLoader::active_task(&loader);
        assert_eq!(task, None);
    }

    #[test]
    fn sets_active_task() {
        let repo_root_tmp = tempdir().expect("Failed to create tempdir");
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let loader = create_loader(repo_root.clone());

        loader
            .file_path
            .ensure_dir()
            .expect("Failed to create directory");

        let preferences = Preferences {
            active_task: Some("web#dev".to_owned()),
            is_task_list_visible: Some(false),
            is_pinned_task_selection: Some(true),
        };

        loader
            .file_path
            .create_with_contents(
                serde_json::to_string_pretty(&preferences)
                    .expect("Failed to serialize preferences"),
            )
            .expect("Failed to create file");

        let task = PreferenceLoader::new(&repo_root).expect("Failed to create PreferenceLoader");
        assert_eq!(task.active_task(), Some("web#dev"));
    }

    #[test]
    fn sets_task_list_visibility() {
        let repo_root_tmp = tempdir().expect("Failed to create tempdir");
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let loader = create_loader(repo_root.clone());

        loader
            .file_path
            .ensure_dir()
            .expect("Failed to create directory");

        let preferences = Preferences {
            active_task: Some("web#dev".to_owned()),
            is_task_list_visible: Some(false),
            is_pinned_task_selection: Some(true),
        };

        loader
            .file_path
            .create_with_contents(
                serde_json::to_string_pretty(&preferences)
                    .expect("Failed to serialize preferences"),
            )
            .expect("Failed to create file");

        let task = PreferenceLoader::new(&repo_root).expect("Failed to create PreferenceLoader");
        assert!(!task.is_task_list_visible());
    }
}
