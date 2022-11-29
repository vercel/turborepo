use strip_ansi_escapes::strip as strip_ansi_escapes;
use thiserror::Error as ThisError;

#[derive(ThisError, Debug)]
pub enum GetDisplayLengthError {
    #[error("Could not strip ANSI escape codes from string")]
    StripError,
    #[error("Could not convert to string")]
    ConvertError,
}

pub fn get_display_length(line: &str) -> Result<usize, GetDisplayLengthError> {
    // strip any ansii escape codes (for color)
    let stripped = strip_ansi_escapes(line);
    if let Ok(stripped) = stripped {
        // convert back to a string
        let stripped = String::from_utf8(stripped);
        if let Ok(stripped) = stripped {
            // count the chars instead of the bytes (for unicode)
            return Ok(stripped.chars().count());
        }
        return Err(GetDisplayLengthError::ConvertError);
    }
    return Err(GetDisplayLengthError::StripError);
}
