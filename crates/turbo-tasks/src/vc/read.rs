use std::{any::Any, marker::PhantomData, mem::ManuallyDrop, pin::Pin, task::Poll};

use anyhow::Result;
use futures::Future;

use super::traits::VcValueType;
use crate::{ReadRawVcFuture, VcCast, VcValueTrait, VcValueTraitCast, VcValueTypeCast};

/// Trait that controls the value returned inside of [`ReadRef`][crate::ReadRef]
/// when `.await`ing a [`Vc<Repr>`][crate::Vc].
///
/// Has two implementations:
/// * [`VcDefaultRead`]
/// * [`VcTransparentRead`]
///
/// `Repr` is the representation type. It is what will be used to
/// serialize/deserialize the value, and it determines the type that the value
/// will be upcasted to for storage.
///
/// [`Self::Target`] is the unwrapped type that will be returned as part of a
/// [`ReadRef`][crate::ReadRef] when `.await`ing a `Vc<Repr>`.
///
/// For most types, `Repr` and `Target` types are the same. When these are the
/// same, `VcDefaultRead` is used.
///
/// In the case of `#[tokio_tasks::value(transparent)]` types, the value is
/// internally stored using the wrapper type, but read as the contained inner
/// type.
///
/// This trait is [sealed][].
///
/// [sealed]: https://rust-lang.github.io/api-guidelines/future-proofing.html
pub trait VcRead<Repr>
where
    Repr: VcValueType,
{
    /// The read target type. This is the type that will be returned as part of
    /// a [`ReadRef`][crate::ReadRef] when `.await`ing a `Vc<T>`.
    ///
    /// For instance, the target of `.await`ing a `Vc<Completion>` will be a
    /// `Completion`. When using `#[turbo_tasks::value(transparent)]`, the
    /// target will be different than the value type.
    type Target;

    /// Convert a reference to a value to a reference to the target type.
    fn repr_to_target_ref(value: &Repr) -> &Self::Target;

    /// Convert a value of the [`Self::Target`] type to the wrapping
    /// [`VcValueType`] that represents it.
    fn target_to_repr(target: Self::Target) -> Repr;

    /// Convert a reference to a target type to a reference to a value.
    fn target_to_repr_ref(target: &Self::Target) -> &Repr;
}

/// Representation for standard `#[turbo_tasks::value]`, where a read returns a
/// reference to the value type.
///
/// Methods such as `target_to_value` are a no-op.
pub struct VcDefaultRead<Repr> {
    _phantom: PhantomData<Repr>,
}

impl<Repr> VcRead<Repr> for VcDefaultRead<Repr>
where
    Repr: VcValueType,
{
    type Target = Repr;

    fn repr_to_target_ref(value: &Repr) -> &Self::Target {
        value
    }

    fn target_to_repr(target: Self::Target) -> Repr {
        target
    }

    fn target_to_repr_ref(target: &Self::Target) -> &Repr {
        target
    }
}

/// Representation for `#[turbo_tasks::value(transparent)]` types, where reads
/// return a reference to the target type.
pub struct VcTransparentRead<Repr, Target> {
    _phantom: PhantomData<(Repr, Target)>,
}

impl<Repr, Target> VcRead<Repr> for VcTransparentRead<Repr, Target>
where
    Repr: VcValueType,
    Target: Any + Send + Sync,
{
    type Target = Target;

    fn repr_to_target_ref(value: &Repr) -> &Self::Target {
        // Safety: the `VcValueType` implementor must guarantee that both `Repr` and
        // `Target` are #[repr(transparent)]. This is guaranteed by the
        // `#[turbo_tasks::value(transparent)]` macro.
        //
        // We can't use `std::mem::transmute` here as it doesn't support generic types.
        // See https://users.rust-lang.org/t/transmute-doesnt-work-on-generic-types/87272/9
        unsafe {
            std::mem::transmute_copy::<ManuallyDrop<&Repr>, &Self::Target>(&ManuallyDrop::new(
                value,
            ))
        }
    }

    fn target_to_repr(target: Self::Target) -> Repr {
        // Safety: see `Self::value_to_target` above.
        unsafe {
            std::mem::transmute_copy::<ManuallyDrop<Self::Target>, Repr>(&ManuallyDrop::new(target))
        }
    }

    fn target_to_repr_ref(target: &Self::Target) -> &Repr {
        // Safety: see `Self::value_to_target` above.
        unsafe {
            std::mem::transmute_copy::<ManuallyDrop<&Self::Target>, &Repr>(&ManuallyDrop::new(
                target,
            ))
        }
    }
}

pub struct ReadVcFuture<T, Cast = VcValueTypeCast<T>>
where
    T: ?Sized,
    Cast: VcCast,
{
    raw: ReadRawVcFuture,
    _phantom_t: PhantomData<T>,
    _phantom_cast: PhantomData<Cast>,
}

impl<T> From<ReadRawVcFuture> for ReadVcFuture<T, VcValueTypeCast<T>>
where
    T: VcValueType,
{
    fn from(raw: ReadRawVcFuture) -> Self {
        Self {
            raw,
            _phantom_t: PhantomData,
            _phantom_cast: PhantomData,
        }
    }
}

impl<T> From<ReadRawVcFuture> for ReadVcFuture<T, VcValueTraitCast<T>>
where
    T: VcValueTrait + ?Sized,
{
    fn from(raw: ReadRawVcFuture) -> Self {
        Self {
            raw,
            _phantom_t: PhantomData,
            _phantom_cast: PhantomData,
        }
    }
}

impl<T, Cast> Future for ReadVcFuture<T, Cast>
where
    T: ?Sized,
    Cast: VcCast,
{
    type Output = Result<Cast::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        // Safety: We never move the contents of `self`
        let raw = unsafe { self.map_unchecked_mut(|this| &mut this.raw) };
        Poll::Ready(std::task::ready!(raw.poll(cx)).and_then(Cast::cast))
    }
}
