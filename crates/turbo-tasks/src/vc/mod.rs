mod cell_mode;
mod read;
mod traits;

use std::{any::Any, marker::PhantomData, ops::Deref};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use self::cell_mode::VcCellMode;
pub use self::{
    cell_mode::{VcCellNewMode, VcCellSharedMode},
    read::{VcDefaultRead, VcRead, VcTransparentRead},
    traits::{Dynamic, TypedForInput, Upcast, VcValueTrait, VcValueType},
};
use crate::{
    debug::{ValueDebug, ValueDebugFormat, ValueDebugFormatString},
    trace::{TraceRawVcs, TraceRawVcsContext},
    CollectiblesFuture, CollectiblesSource, ConcreteTaskInput, FromTaskInput, RawVc,
    ReadRawVcFuture, ReadRef, ResolveTypeError,
};

/// A Value Cell (`Vc` for short) is a reference to a memoized computation
/// result stored on the heap or in persistent cache, depending on the
/// Turbo Engine backend implementation.
///
/// In order to get a reference to the pointed value, you need to `.await` the
/// [`Vc<T>`] to get a [`ReadRef<T>`]:
///
/// ```
/// let some_vc: Vc<T>;
/// let some_ref: ReadRef<T> = some_vc.await?;
/// some_ref.some_method_on_t();
/// ```
pub struct Vc<T>
where
    T: ?Sized,
{
    // TODO(alexkirsz) Should be private (or undocumented), but turbo-tasks-memory needs it to be
    // accessible.
    pub node: RawVc,
    pub(crate) _t: PhantomData<T>,
}

pub struct VcDeref<T>
where
    T: ?Sized,
{
    _t: PhantomData<T>,
}

trait Impossible {}

macro_rules! hide_methods {
    ($($name:ident)*) => {
        impl<T> VcDeref<T>
        where
            T: ?Sized,
        {
            $(
                #[doc(hidden)]
                #[allow(unused)]
                #[deprecated = "This is not the method you are looking for."]
                pub fn $name(self) {}
            )*
        }
    };
}

hide_methods!(
    add
    addr
    align_offset
    as_mut
    as_mut_ptr
    as_ptr
    as_ref
    as_uninit_mut
    as_uninit_ref
    as_uninit_slice
    as_uninit_slice_mut
    byte_add
    byte_offset
    byte_offset_from
    byte_sub
    cast
    cast_const
    cast_mut
    copy_from
    copy_from_nonoverlapping
    copy_to
    copy_to_nonoverlapping
    drop_in_place
    expose_addr
    from_bits
    get_unchecked
    get_unchecked_mut
    guaranteed_eq
    guaranteed_ne
    is_aligned
    is_aligned_to
    is_empty
    is_null
    len
    map_addr
    mask
    offset
    offset_from
    read
    read_unaligned
    read_volatile
    replace
    split_at_mut
    split_at_mut_unchecked
    sub
    sub_ptr
    swap
    to_bits
    to_raw_parts
    with_addr
    with_metadata_of
    wrapping_add
    wrapping_byte_add
    wrapping_byte_offset
    wrapping_byte_sub
    wrapping_offset
    wrapping_sub
    write
    write_bytes
    write_unaligned
    write_volatile
);

// Call this macro for all the applicable methods above:

#[doc(hidden)]
impl<T> Deref for VcDeref<T>
where
    T: ?Sized,
{
    // `*const T` or `*mut T` would be enough here, but from an abundance of
    // caution, we use `*const *mut *const T` to make sure there will never be an
    // applicable method.
    type Target = *const *mut *const T;

    fn deref(&self) -> &Self::Target {
        extern "C" {
            #[link_name = "\n\nERROR: you tried to dereference a `Vc<T>`\n"]
            fn trigger() -> !;
        }

        unsafe { trigger() };
    }
}

// This is the magic that makes `Vc<T>` accept `self: Vc<Self>` methods through
// `arbitrary_self_types`, while not allowing any other receiver type:
// * `Vc<T>` dereferences to `*const *mut *const T`, which means that it is
//   valid under the `arbitrary_self_types` rules.
// * `*const *mut *const T` is not a valid receiver for any attribute access on
//   `T`, which means that the only applicable items will be the methods
//   declared on `self: Vc<Self>`.
//
// If we had used `type Target = T` instead, `vc_t.some_attr_defined_on_t` would
// have been accepted by the compiler.
#[doc(hidden)]
impl<T> Deref for Vc<T>
where
    T: ?Sized,
{
    type Target = VcDeref<T>;

    fn deref(&self) -> &Self::Target {
        extern "C" {
            #[link_name = "\n\nERROR: you tried to dereference a `Vc<T>`\n"]
            fn trigger() -> !;
        }

        unsafe { trigger() };
    }
}

impl<T> Copy for Vc<T> where T: ?Sized {}

unsafe impl<T> Send for Vc<T> where T: ?Sized {}
unsafe impl<T> Sync for Vc<T> where T: ?Sized {}

impl<T> Clone for Vc<T>
where
    T: ?Sized,
{
    fn clone(&self) -> Self {
        Self {
            node: self.node.clone(),
            _t: PhantomData,
        }
    }
}

impl<T> core::hash::Hash for Vc<T>
where
    T: ?Sized,
{
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.node.hash(state);
    }
}

impl<T> PartialEq<Vc<T>> for Vc<T>
where
    T: ?Sized,
{
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
    }
}

impl<T> Eq for Vc<T> where T: ?Sized {}

impl<T> PartialOrd<Vc<T>> for Vc<T>
where
    T: ?Sized,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.node.partial_cmp(&other.node)
    }
}

impl<T> Ord for Vc<T>
where
    T: ?Sized,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.node.cmp(&other.node)
    }
}

impl<T> Serialize for Vc<T>
where
    T: ?Sized,
{
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.node.serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for Vc<T>
where
    T: ?Sized,
{
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Vc {
            node: RawVc::deserialize(deserializer)?,
            _t: PhantomData,
        })
    }
}

impl<T> Vc<T>
where
    T: VcValueType,
{
    #[doc(hidden)]
    pub fn cell_private(inner: <T::Read as VcRead<T>>::Target) -> Self {
        <T::CellMode as cell_mode::VcCellMode<T>>::cell(inner)
    }
}

impl<T, Inner> Vc<T>
where
    T: VcValueType<Read = VcTransparentRead<T, Inner>>,
    Inner: Any + Send + Sync,
{
    pub fn cell(inner: Inner) -> Self {
        <T::CellMode as VcCellMode<T>>::cell(inner)
    }
}

impl<T> Vc<T>
where
    T: ?Sized,
{
    pub fn upcast<K>(&self) -> Vc<K>
    where
        T: Upcast<K>,
        K: VcValueTrait + ?Sized,
    {
        Vc {
            node: self.node,
            _t: PhantomData,
        }
    }
}

impl<T> Vc<T>
where
    T: ?Sized,
{
    /// Resolve the reference until it points to a cell directly.
    ///
    /// Resolving will wait for task execution to be finished, so that the
    /// returned `Vc` points to a cell that stores a value.
    ///
    /// Resolving is necessary to compare identities of `Vc`s.
    ///
    /// This is async and will rethrow any fatal error that happened during task
    /// execution.
    pub async fn resolve(self) -> Result<Self> {
        Ok(Self {
            node: self.node.resolve().await?,
            _t: PhantomData,
        })
    }

    /// Resolve the reference until it points to a cell directly in a strongly
    /// consistent way.
    ///
    /// Resolving will wait for task execution to be finished, so that the
    /// returned Vc points to a cell that stores a value.
    ///
    /// Resolving is necessary to compare identities of Vcs.
    ///
    /// This is async and will rethrow any fatal error that happened during task
    /// execution.
    pub async fn resolve_strongly_consistent(self) -> Result<Self> {
        Ok(Self {
            node: self.node.resolve_strongly_consistent().await?,
            _t: PhantomData,
        })
    }
}

impl<T> Vc<T>
where
    T: VcValueTrait + ?Sized,
{
    pub async fn resolve_upcast<K>(sub_trait_or_type: &Vc<K>) -> Result<Self, ResolveTypeError>
    where
        K: ?Sized,
        K: Upcast<T>,
    {
        Ok(Vc::<T>::try_resolve_upcast(sub_trait_or_type)
            .await?
            .expect("resolving trait should always return a value when the Upcast impl is correct"))
    }

    pub async fn try_resolve_upcast<K>(
        sub_trait_or_type: &Vc<K>,
    ) -> Result<Option<Self>, ResolveTypeError>
    where
        K: ?Sized,
    {
        let raw_vc: RawVc = sub_trait_or_type.node;
        let raw_vc = raw_vc
            .resolve_trait(<T as VcValueTrait>::get_trait_type_id())
            .await?;
        Ok(raw_vc.map(|raw_vc| Vc {
            node: raw_vc,
            _t: PhantomData,
        }))
    }
}

impl<T> CollectiblesSource for Vc<T>
where
    T: ?Sized,
{
    fn take_collectibles<Vt: VcValueTrait>(self) -> CollectiblesFuture<Vt> {
        self.node.take_collectibles()
    }

    fn peek_collectibles<Vt: VcValueTrait>(self) -> CollectiblesFuture<Vt> {
        self.node.peek_collectibles()
    }
}

impl<T> FromTaskInput<'_> for Vc<T>
where
    T: ?Sized,
{
    type Error = anyhow::Error;

    fn try_from(value: &ConcreteTaskInput) -> Result<Self, Self::Error> {
        Ok(Self {
            node: value.try_into()?,
            _t: PhantomData,
        })
    }
}

impl<T> From<RawVc> for Vc<T>
where
    T: ?Sized,
{
    fn from(node: RawVc) -> Self {
        Self {
            node,
            _t: PhantomData,
        }
    }
}

impl<T> From<Vc<T>> for ConcreteTaskInput
where
    T: ?Sized,
{
    fn from(node_ref: Vc<T>) -> Self {
        node_ref.node.into()
    }
}

impl<T> From<&Vc<T>> for ConcreteTaskInput
where
    T: ?Sized,
{
    fn from(node_ref: &Vc<T>) -> Self {
        node_ref.node.clone().into()
    }
}

impl<T> TraceRawVcs for Vc<T>
where
    T: ?Sized,
{
    fn trace_raw_vcs(&self, context: &mut TraceRawVcsContext) {
        TraceRawVcs::trace_raw_vcs(&self.node, context);
    }
}

impl<T> ValueDebugFormat for Vc<T>
where
    T: ?Sized,
    for<'a> T: Upcast<&'a dyn ValueDebug>,
{
    fn value_debug_format(&self, depth: usize) -> ValueDebugFormatString {
        ValueDebugFormatString::Async(Box::pin(async move {
            Ok({
                let vc_value_debug = self.upcast::<&dyn ValueDebug>();
                vc_value_debug.dbg_depth(depth).await?.to_string()
            })
        }))
    }
}

impl<T> std::future::IntoFuture for Vc<T>
where
    T: VcValueType,
{
    type Output = anyhow::Result<ReadRef<T>>;
    type IntoFuture = ReadRawVcFuture<T>;
    fn into_future(self) -> Self::IntoFuture {
        self.node.into_read::<T>()
    }
}

impl<T> std::future::IntoFuture for &Vc<T>
where
    T: VcValueType,
{
    type Output = <Vc<T> as std::future::IntoFuture>::Output;
    type IntoFuture = <Vc<T> as std::future::IntoFuture>::IntoFuture;
    fn into_future(self) -> Self::IntoFuture {
        (*self).into_future()
    }
}

impl<T> Vc<T>
where
    T: VcValueType,
{
    /// Returns a strongly consistent read of the value. This ensures that all
    /// internal tasks are finished before the read is returned.
    #[must_use]
    pub fn strongly_consistent(self) -> ReadRawVcFuture<T> {
        self.node.into_strongly_consistent_read::<T>()
    }
}
