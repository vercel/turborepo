use turbopath::AbsoluteSystemPath;

pub struct Preferences {}

impl Preferences {
    pub fn write_preferences(repo_root: &AbsoluteSystemPath) {
        let preferences_file = repo_root.join_components(&[".turbo", "preferences", "tui.json"]);
        println!("TODO: save preferences to {:?}", preferences_file);
    }
}
