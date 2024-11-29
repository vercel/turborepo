use std::{
    fs::{self, File},
    io::{BufReader, Write},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use turbopath::AbsoluteSystemPathBuf;

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

impl Preferences {
    pub fn update_preference(
        repo_root: &AbsoluteSystemPathBuf,
        field: PreferenceFields,
        new_value: Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
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

    pub fn read_preferences(
        repo_root: &AbsoluteSystemPathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let preferences_file = repo_root.join_components(&[".turbo", "preferences", "tui.json"]);

        fn read_from_json(path: &str) -> Result<Preferences, Box<dyn std::error::Error>> {
            let file = File::open(path)?;
            let reader = BufReader::new(file);

            let preferences: Preferences = serde_json::from_reader(reader)?;

            Ok(preferences)
        }

        read_from_json(preferences_file.as_std_path().to_str().unwrap())
    }
}
