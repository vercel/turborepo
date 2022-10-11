use std::env;

pub enum ColorMode {
    Undefined,
    Suppressed,
    Forced,
}

impl ColorMode {
    pub fn get_from_env() -> Self {
        match env::var("FORCE_COLOR") {
            Err(_) => ColorMode::Undefined,
            Ok(force_color) => match force_color.as_str() {
                "false" | "0" => ColorMode::Suppressed,
                "true" | "1" | "2" | "3" => ColorMode::Forced,
                _ => ColorMode::Undefined,
            },
        }
    }
}
