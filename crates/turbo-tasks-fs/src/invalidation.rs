use std::borrow::Cow;

use indexmap::IndexSet;
use turbo_tasks::{InvalidationReason, InvalidationReasonType};

pub struct WatchChange {
    pub path: String,
}

impl InvalidationReason for WatchChange {
    fn description(&self) -> Cow<'static, str> {
        format!("{} changed", self.path).into()
    }
    fn merge_info(&self) -> Option<(&'static dyn InvalidationReasonType, Cow<'static, str>)> {
        Some((&WATCH_CHANGE_TYPE, self.path.clone().into()))
    }
}

struct WatchChangeType {
    _non_zero_sized: u8,
}

static WATCH_CHANGE_TYPE: WatchChangeType = WatchChangeType { _non_zero_sized: 0 };

impl InvalidationReasonType for WatchChangeType {
    fn description(&self, merge_data: &IndexSet<Cow<'static, str>>) -> Cow<'static, str> {
        format!(
            "{} files changed (e. g. {})",
            merge_data.len(),
            merge_data[0]
        )
        .into()
    }
}

pub struct WatchStart {
    pub name: String,
}

impl InvalidationReason for WatchStart {
    fn description(&self) -> Cow<'static, str> {
        format!("{} started watching", self.name).into()
    }
    fn merge_info(&self) -> Option<(&'static dyn InvalidationReasonType, Cow<'static, str>)> {
        Some((&WATCH_START_TYPE, self.name.clone().into()))
    }
}

struct WatchStartType {
    _non_zero_sized: u8,
}

static WATCH_START_TYPE: WatchStartType = WatchStartType { _non_zero_sized: 0 };

impl InvalidationReasonType for WatchStartType {
    fn description(&self, merge_data: &IndexSet<Cow<'static, str>>) -> Cow<'static, str> {
        format!(
            "{} directories started watching (e. g. {})",
            merge_data.len(),
            merge_data[0]
        )
        .into()
    }
}
