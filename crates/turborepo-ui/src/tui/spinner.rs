use std::time::{Duration, Instant};

const SPINNER_FRAMES: &[&str] = ["»"].as_slice();
// const SPINNER_FRAMES: &[&str] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇",
// "⠏"].as_slice();
const FRAMERATE: Duration = Duration::from_millis(80);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpinnerState {
    frame: usize,
    last_render: Option<Instant>,
}

impl SpinnerState {
    pub fn new() -> Self {
        Self {
            frame: 0,
            last_render: None,
        }
    }

    pub fn update(&mut self) {
        if let Some(last_render) = self.last_render {
            if last_render.elapsed() > FRAMERATE {
                self.frame = (self.frame + 1) % SPINNER_FRAMES.len();
                self.last_render = Some(Instant::now());
            }
        } else {
            self.last_render = Some(Instant::now());
        }
    }

    pub fn current(&self) -> &'static str {
        SPINNER_FRAMES[self.frame]
    }
}

impl Default for SpinnerState {
    fn default() -> Self {
        Self::new()
    }
}

// Removed with iteration to double arrow symbol
// #[cfg(test)]
// mod test {
//     use super::*;
//
//     #[test]
//     fn test_inital_update() {
//         let mut spinner = SpinnerState::new();
//         assert!(spinner.last_render.is_none());
//         assert_eq!(spinner.frame, 0);
//         spinner.update();
//         assert!(spinner.last_render.is_some());
//         assert_eq!(spinner.frame, 0, "initial update doesn't move frame");
//     }
//
//     Removed with change to double arrow
//     #[test]
//     fn test_frame_update() {
//         let mut spinner = SpinnerState::new();
//         // set last update to time that happened far before the spinner
// should increment         let prev_render = Instant::now() - (FRAMERATE * 2);
//         spinner.last_render = Some(prev_render);
//         assert_eq!(spinner.frame, 0);
//         spinner.update();
//         assert_eq!(spinner.frame, 1);
//         let last_render = spinner.last_render.unwrap();
//         assert!(prev_render < last_render, "last render should be updated");
//     }
// }
