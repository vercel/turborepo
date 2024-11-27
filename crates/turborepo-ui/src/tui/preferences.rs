use std::{
    fs::{self, File},
    io::{BufReader, Write},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

#[derive(Serialize, Deserialize, Debug)]
pub struct Preferences {
    pub is_task_list_visible: bool,
}

fn save_to_json(
    preferences: &Preferences,
    path: AbsoluteSystemPathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(preferences)?;
    let mut file = File::create(path.as_std_path())?;
    file.write_all(json.as_bytes())?;

    Ok(())
}

fn update_json_field(
    file_path: &str,
    field: &str,
    new_value: Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let json_string = fs::read_to_string(file_path)?;
    let mut json: Value = serde_json::from_str(&json_string)?;

    json[field] = new_value;
    let updated_json_string = serde_json::to_string_pretty(&json)?;

    let mut file = fs::File::create(file_path)?;
    file.write_all(updated_json_string.as_bytes())?;

    Ok(())
}

fn read_from_json(path: &str) -> Result<Preferences, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let person: Preferences = serde_json::from_reader(reader)?;

    Ok(person)
}

impl Preferences {
    pub fn write_preferences(
        repo_root: &AbsoluteSystemPath,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let preferences_dir = repo_root.join_components(&[".turbo", "preferences"]);
        let preferences_file = preferences_dir.join_component("tui.json");

        // Create the directory structure if it doesn't exist
        fs::create_dir_all(preferences_dir.as_std_path())?;

        save_to_json(
            &Preferences {
                is_task_list_visible: true,
            },
            preferences_file,
        )
        .unwrap();

        Ok(())
    }
}
