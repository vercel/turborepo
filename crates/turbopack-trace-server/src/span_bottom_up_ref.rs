use std::fmt::{Debug, Formatter};

use crate::{
    span::SpanBottomUp,
    span_ref::SpanRef,
    store::{SpanId, Store},
};

pub struct SpanBottomUpRef<'a> {
    pub(crate) bottom_up: &'a SpanBottomUp,
    pub(crate) store: &'a Store,
}

impl<'a> SpanBottomUpRef<'a> {
    pub fn id(&self) -> SpanId {
        unsafe { SpanId::new_unchecked((self.bottom_up.example_span.get() << 1) | 1) }
    }

    fn first_span(&self) -> SpanRef<'a> {
        SpanRef {
            span: &self.store.spans[self.bottom_up.self_spans[0].get()],
            store: self.store,
        }
    }

    fn example_span(&self) -> SpanRef<'a> {
        SpanRef {
            span: &self.store.spans[self.bottom_up.example_span.get()],
            store: self.store,
        }
    }

    pub fn spans(&'a self) -> impl Iterator<Item = SpanRef<'a>> + 'a {
        self.bottom_up.self_spans.iter().map(move |span| SpanRef {
            span: &self.store.spans[span.get()],
            store: self.store,
        })
    }

    pub fn count(&self) -> usize {
        self.bottom_up.self_spans.len()
    }

    pub fn group_name(&self) -> &'a str {
        self.first_span().group_name()
    }

    pub fn nice_name(&self) -> (&'a str, &'a str) {
        if self.count() == 1 {
            self.example_span().nice_name()
        } else {
            ("", self.example_span().group_name())
        }
    }

    pub fn children(&self) -> impl Iterator<Item = SpanBottomUpRef<'a>> + 'a {
        self.bottom_up
            .children
            .values()
            .map(|bottom_up| SpanBottomUpRef {
                bottom_up,
                store: self.store,
            })
    }

    pub fn max_depth(&self) -> u32 {
        *self.bottom_up.max_depth.get_or_init(|| {
            self.children()
                .map(|bottom_up| bottom_up.max_depth() + 1)
                .max()
                .unwrap_or(0)
        })
    }

    pub fn corrected_self_time(&self) -> u64 {
        *self
            .bottom_up
            .corrected_self_time
            .get_or_init(|| self.spans().map(|span| span.corrected_self_time()).sum())
    }

    pub fn self_allocations(&self) -> u64 {
        *self
            .bottom_up
            .self_allocations
            .get_or_init(|| self.spans().map(|span| span.self_allocations()).sum())
    }

    pub fn self_deallocations(&self) -> u64 {
        *self
            .bottom_up
            .self_deallocations
            .get_or_init(|| self.spans().map(|span| span.self_deallocations()).sum())
    }

    pub fn self_persistent_allocations(&self) -> u64 {
        *self.bottom_up.self_persistent_allocations.get_or_init(|| {
            self.spans()
                .map(|span| span.self_persistent_allocations())
                .sum()
        })
    }

    pub fn self_allocation_count(&self) -> u64 {
        *self
            .bottom_up
            .self_allocation_count
            .get_or_init(|| self.spans().map(|span| span.self_allocation_count()).sum())
    }
}

impl<'a> Debug for SpanBottomUpRef<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanBottomUpRef")
            .field("group_name", &self.group_name())
            .field("max_depth", &self.max_depth())
            .field("corrected_self_time", &self.corrected_self_time())
            .field("self_allocations", &self.self_allocations())
            .field("self_deallocations", &self.self_deallocations())
            .field(
                "self_persistent_allocations",
                &self.self_persistent_allocations(),
            )
            .field("self_allocation_count", &self.self_allocation_count())
            .finish()
    }
}
