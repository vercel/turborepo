use turbo_tasks::Vc;

use super::{available_modules::AvailableAssets, ChunkableModule};

#[turbo_tasks::value(serialization = "auto_for_input")]
#[derive(PartialOrd, Ord, Hash, Clone, Copy, Debug)]
pub enum AvailabilityInfo {
    /// Availability of modules is not tracked
    Untracked,
    /// Availablility of modules is tracked, but no modules are available
    Root,
    /// There are modules already available.
    Complete {
        available_modules: Vc<AvailableAssets>,
    },
}

impl AvailabilityInfo {
    pub fn available_modules(&self) -> Option<Vc<AvailableAssets>> {
        match self {
            Self::Untracked => None,
            Self::Root => None,
            Self::Complete {
                available_modules, ..
            } => Some(*available_modules),
        }
    }

    pub fn with_modules(self, modules: Vec<Vc<Box<dyn ChunkableModule>>>) -> Self {
        match self {
            AvailabilityInfo::Untracked => AvailabilityInfo::Untracked,
            AvailabilityInfo::Root => AvailabilityInfo::Complete {
                available_modules: AvailableAssets::new(modules),
            },
            AvailabilityInfo::Complete { available_modules } => AvailabilityInfo::Complete {
                available_modules: available_modules.with_modules(modules),
            },
        }
    }
}
