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
        self.flush_to_disk()
    }

    pub fn active_task(&self) -> Option<&str> {
        let active_task = self.config.active_task.as_deref()?;
        Some(active_task)
    }

    pub fn set_active_task(&mut self, value: Option<String>) -> Result<(), Error> {
        self.config.active_task = value;
        self.flush_to_disk()
    }

    fn flush_to_disk(&self) -> Result<(), Error> {
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
