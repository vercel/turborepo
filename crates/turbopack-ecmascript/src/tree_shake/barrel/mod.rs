pub struct BarrelOptimizer {}

/// The type of an ECMAScript file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Normal,
    Barrel,
    Full,
}

impl FileType {}
