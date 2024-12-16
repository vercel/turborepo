use std::{
    fs::{self, File},
    io::{BufReader, Write},
};

use serde::{Deserialize, Serialize};
use serde_json::{from_reader, json, Value};
use turbopath::AbsoluteSystemPathBuf;

use super::task::TasksByStatus;

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

    pub fn is_pinned_task_selection(&self) -> bool {
        self.config.is_pinned_task_selection.unwrap_or(true)
    }

    pub fn set_active_task(&mut self, value: Option<String>) -> Result<(), Error> {
        self.config.active_task = value;
        self.flush_to_disk()
    }

    fn flush_to_disk(&self) -> Result<(), Error> {
        self.file_path.ensure_dir();
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

#[derive(Debug)]
pub enum PreferenceFields {
    IsTaskListVisible,
    ActiveTask,
    PinnedTaskSelection,
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

fn read_json(path: &AbsoluteSystemPathBuf) -> Preferences {
    File::open(path)
        .ok()
        .and_then(|file| from_reader(BufReader::new(file)).ok())
        .unwrap_or_default()
}

impl Preferences {
    pub fn update_preference(
        repo_root: &AbsoluteSystemPathBuf,
        field: PreferenceFields,
        new_value: Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // TODO: Clean these up, should be taken from constants
        let preferences_dir = repo_root.join_components(&[".turbo", "preferences"]);
        let preferences_file = repo_root.join_components(&[".turbo", "preferences", "tui.json"]);

        fs::create_dir_all(preferences_dir.as_std_path())?;

        let mut json: Value = if preferences_file.exists() {
            let json_string = fs::read_to_string(&preferences_file)?;
            serde_json::from_str(&json_string)?
        } else {
            json!({})
        };

        // TODO: Is this really how to do this? No way, right?
        match field {
            PreferenceFields::IsTaskListVisible => {
                json["is_task_list_visible"] = new_value;
            }
            PreferenceFields::ActiveTask => {
                json["active_task"] = new_value;
            }
            PreferenceFields::PinnedTaskSelection => {
                json["is_pinned_task_selection"] = new_value;
            }
        }

        let updated_json_string = serde_json::to_string_pretty(&json)?;

        let mut file = fs::File::create(&preferences_file)?;
        file.write_all(updated_json_string.as_bytes())?;

        Ok(())
    }

    pub fn read_pinned_task_state(repo_root: &AbsoluteSystemPathBuf) -> bool {
        let preferences_file = repo_root.join_components(TUI_PREFERENCES_PATH_COMPONENTS);

        read_json(&preferences_file)
            .is_pinned_task_selection
            .unwrap_or(false)
    }

    pub fn read_task_list_visibility(repo_root: &AbsoluteSystemPathBuf) -> bool {
        let preferences_file = repo_root.join_components(TUI_PREFERENCES_PATH_COMPONENTS);

        read_json(&preferences_file)
            .is_task_list_visible
            .unwrap_or(true)
    }

    pub fn get_selected_task_index(
        repo_root: &AbsoluteSystemPathBuf,
        tasks_by_status: &TasksByStatus,
    ) -> usize {
        let preferences_file = repo_root.join_components(TUI_PREFERENCES_PATH_COMPONENTS);

        let selected_task_name = read_json(&preferences_file)
            .active_task
            .unwrap_or("".to_string());

        match tasks_by_status
            .task_names_in_displayed_order()
            .position(|task_name| *task_name == selected_task_name)
        {
            Some(index) => index,
            None => {
                let _ = Self::update_preference(
                    repo_root,
                    PreferenceFields::PinnedTaskSelection,
                    serde_json::Value::Bool(false),
                );

                let _ = Self::update_preference(
                    repo_root,
                    PreferenceFields::ActiveTask,
                    serde_json::Value::String("".to_string()),
                );
                0
            }
        }
    }
}
