const SPINNER_FRAMES: &[&str] = ["»"].as_slice();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpinnerState {
    frame: usize,
}

impl SpinnerState {
    pub fn new() -> Self {
        Self { frame: 0 }
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
