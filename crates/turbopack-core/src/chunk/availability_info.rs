use std::ops::{Add, AddAssign};

use turbo_tasks::Vc;

use super::available_modules::AvailableAssets;
use crate::module::Module;

#[turbo_tasks::value(shared)]
#[derive(Copy, Clone, Debug)]
pub enum AvailabilityInfoNeeds {
    None,
    Root,
    AvailableModules,
    Complete,
}

impl Add for AvailabilityInfoNeeds {
    type Output = AvailabilityInfoNeeds;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::None, rhs) => rhs,
            (lhs, Self::None) => lhs,
            (Self::Complete, _) | (_, Self::Complete) => Self::Complete,
            (Self::Root, Self::Root) => Self::Root,
            (Self::AvailableModules, Self::AvailableModules) => Self::AvailableModules,
            (Self::Root, Self::AvailableModules) | (Self::AvailableModules, Self::Root) => {
                Self::Complete
            }
        }
    }
}

impl AddAssign for AvailabilityInfoNeeds {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

#[turbo_tasks::value(serialization = "auto_for_input")]
#[derive(PartialOrd, Ord, Hash, Clone, Copy, Debug)]
pub enum AvailabilityInfo {
    Untracked,
    Root {
        current_availability_root: Vc<Box<dyn Module>>,
    },
    Complete {
        available_modules: Vc<AvailableAssets>,
        current_availability_root: Vc<Box<dyn Module>>,
    },
    OnlyAvailableModules {
        available_modules: Vc<AvailableAssets>,
    },
}

impl AvailabilityInfo {
    pub fn current_availability_root(&self) -> Option<Vc<Box<dyn Module>>> {
        match self {
            Self::Untracked => None,
            Self::Root {
                current_availability_root,
            } => Some(*current_availability_root),
            Self::Complete {
                current_availability_root,
                ..
            } => Some(*current_availability_root),
            Self::OnlyAvailableModules { .. } => None,
        }
    }

    pub fn available_modules(&self) -> Option<Vc<AvailableAssets>> {
        match self {
            Self::Untracked => None,
            Self::Root { .. } => None,
            Self::Complete {
                available_modules, ..
            } => Some(*available_modules),
            Self::OnlyAvailableModules {
                available_modules, ..
            } => Some(*available_modules),
        }
    }

    pub fn reduce_to_needs(self, needs: AvailabilityInfoNeeds) -> Self {
        match needs {
            AvailabilityInfoNeeds::None => Self::Untracked,
            AvailabilityInfoNeeds::Root => match self {
                Self::Untracked => Self::Untracked,
                Self::OnlyAvailableModules { .. } => Self::Untracked,
                Self::Root {
                    current_availability_root,
                } => Self::Root {
                    current_availability_root,
                },
                Self::Complete {
                    current_availability_root,
                    ..
                } => Self::Root {
                    current_availability_root,
                },
            },
            AvailabilityInfoNeeds::AvailableModules => match self {
                Self::Untracked => Self::Untracked,
                Self::Root { .. } => Self::Untracked,
                Self::Complete {
                    available_modules,
                    current_availability_root,
                } => Self::Complete {
                    available_modules,
                    current_availability_root,
                },
                Self::OnlyAvailableModules { .. } => Self::Untracked,
            },
            AvailabilityInfoNeeds::Complete => self,
        }
    }
}
