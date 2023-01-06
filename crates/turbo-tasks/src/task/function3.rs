use std::{future::Future, marker::PhantomData, pin::Pin};

use anyhow::{bail, Context, Result};

use super::{ConcreteTaskInput, TaskInput, TaskOutput};
use crate::{RawVc, Vc, VcRead, VcValueType};

pub type NativeTaskFuture = Pin<Box<dyn Future<Output = Result<RawVc>> + Send>>;
pub type NativeTaskFn = Box<dyn Fn() -> NativeTaskFuture + Send + Sync>;

pub trait TaskFn: Send + Sync + 'static {
    fn functor(&self, inputs: &[ConcreteTaskInput]) -> Result<NativeTaskFn>;
}

pub struct AsFunction<F>(pub F);

pub struct AsAsyncFunction<F>(pub F);
pub struct AsMethod<F>(pub F);
pub struct AsAsyncMethod<F>(pub F);

macro_rules! task_fn_impl {
    ( $( $arg:ident )* ) => {
        impl<$($arg,)* Output> TaskFn for AsFunction<fn($($arg),*) -> Output>
        where
            $($arg: TaskInput + 'static,)*
            Output: TaskOutput + 'static,
        {
            #[allow(non_snake_case)]
            fn functor(&self, inputs: &[ConcreteTaskInput]) -> Result<NativeTaskFn> {
                let this = self.0;

                let mut iter = inputs.iter();

                $(
                    let $arg = iter.next().context(format!("task is missing argument {}", stringify!($arg)))?;
                )*

                if iter.next().is_some() {
                    bail!("task was called with too many arguments");
                }

                $(
                    let $arg = $arg::try_from_concrete($arg)?;
                )*

                Ok(Box::new(move || {
                    $(
                        let $arg = $arg.clone();
                    )*

                    Box::pin(async move {
                        Output::try_into_raw_vc((this)($($arg),*))
                    })
                }))
            }
        }

        impl<$($arg,)* FutureOutput, Output> TaskFn for AsAsyncFunction<fn($($arg),*) -> FutureOutput>
        where
            $($arg: TaskInput + 'static,)*
            FutureOutput: Future<Output = Output> + Send + 'static,
            Output: TaskOutput + 'static,
        {
            #[allow(non_snake_case)]
            fn functor(&self, inputs: &[ConcreteTaskInput]) -> Result<NativeTaskFn> {
                let this = self.0;

                let mut iter = inputs.iter();

                $(
                    let $arg = iter.next().context(format!("task is missing argument {}", stringify!($arg)))?;
                )*

                if iter.next().is_some() {
                    bail!("task was called with too many arguments");
                }

                $(
                    let $arg = $arg::try_from_concrete($arg)?;
                )*

                Ok(Box::new(move || {
                    $(
                        let $arg = $arg.clone();
                    )*

                    Box::pin(async move {
                        Output::try_into_raw_vc((this)($($arg),*).await)
                    })
                }))
            }
        }

        impl<Recv, $($arg,)* Output> TaskFn for AsMethod<fn(&Recv, $($arg),*) -> Output>
        where
            Recv: VcValueType,
            Vc<Recv>: TaskInput + 'static,
            $($arg: TaskInput + 'static,)*
            Output: TaskOutput + 'static,
        {
            #[allow(non_snake_case)]
            fn functor(&self, inputs: &[ConcreteTaskInput]) -> Result<NativeTaskFn> {
                let this = self.0;

                let mut iter = inputs.iter();

                let recv = iter.next().context("task is missing receiver")?;
                $(
                    let $arg = iter.next().context(format!("task is missing argument {}", stringify!($arg)))?;
                )*

                if iter.next().is_some() {
                    bail!("task was called with too many arguments");
                }

                let recv = Vc::<Recv>::try_from_concrete(recv)?;
                $(
                    let $arg = $arg::try_from_concrete($arg)?;
                )*

                Ok(Box::new(move || {
                    let recv = recv.clone();
                    $(
                        let $arg = $arg.clone();
                    )*

                    Box::pin(async move {
                        let recv = recv.await?;
                        let recv = <<Recv as VcValueType>::Read as VcRead<Recv>>::target_to_value_ref(&*recv);
                        Output::try_into_raw_vc((this)(recv, $($arg),*))
                    })
                }))
            }
        }

        impl<Recv, $($arg,)* FutureOutput, Output> TaskFn for AsAsyncMethod<fn(&Recv, $($arg),*) -> FutureOutput>
        where
            Recv: VcValueType,
            Vc<Recv>: TaskInput + 'static,
            <<Recv as VcValueType>::Read as VcRead<Recv>>::Target: Send + Sync,
            $($arg: TaskInput + 'static,)*
            FutureOutput: Future<Output = Output> + Send + 'static,
            Output: TaskOutput + 'static,
        {
            #[allow(non_snake_case)]
            fn functor(&self, inputs: &[ConcreteTaskInput]) -> Result<NativeTaskFn> {
                let this = self.0;

                let mut iter = inputs.iter();

                let recv = iter.next().context("task is missing receiver")?;
                $(
                    let $arg = iter.next().context(format!("task is missing argument {}", stringify!($arg)))?;
                )*

                if iter.next().is_some() {
                    bail!("task was called with too many arguments");
                }

                let recv = Vc::<Recv>::try_from_concrete(recv)?;
                $(
                    let $arg = $arg::try_from_concrete($arg)?;
                )*

                Ok(Box::new(move || {
                    let recv = recv.clone();
                    $(
                        let $arg = $arg.clone();
                    )*

                    Box::pin(async move {
                        let recv = recv.await?;
                        let recv = <<Recv as VcValueType>::Read as VcRead<Recv>>::target_to_value_ref(&*recv);
                        Output::try_into_raw_vc((this)(recv, $($arg),*).await)
                    })
                }))
            }
        }
    };
}

task_fn_impl! {}
task_fn_impl! { A1 }
task_fn_impl! { A1 A2 }
task_fn_impl! { A1 A2 A3 }
task_fn_impl! { A1 A2 A3 A4 }
task_fn_impl! { A1 A2 A3 A4 A5 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 A7 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 A7 A8 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 A7 A8 A9 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 A7 A8 A9 A10 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 A7 A8 A9 A10 A11 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 A7 A8 A9 A10 A11 A12 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 A7 A8 A9 A10 A11 A12 A13 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 A7 A8 A9 A10 A11 A12 A13 A14 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 A7 A8 A9 A10 A11 A12 A13 A14 A15 }
task_fn_impl! { A1 A2 A3 A4 A5 A6 A7 A8 A9 A10 A11 A12 A13 A14 A15 A16 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_fn() {
        fn no_args() -> crate::Vc<i32> {
            todo!()
        }

        fn one_arg(_a: i32) -> crate::Vc<i32> {
            todo!()
        }

        fn with_recv(_a: &i32) -> crate::Vc<i32> {
            todo!()
        }

        fn with_recv_and_str(_a: &i32, s: String) -> crate::Vc<i32> {
            todo!()
        }

        async fn async_with_recv_and_str(_a: &i32, s: String) -> crate::Vc<i32> {
            todo!()
        }

        // fn accepts_task_fn<F>(_task_fn: F)
        // where
        //     F: TaskFn,
        // {
        // }

        // accepts_task_fn(AsFunction(no_args as fn() -> _));
        // accepts_task_fn(AsFunction(one_arg as fn(_) -> _));
        // accepts_task_fn(AsMethod(with_recv as fn(&'_ _) -> _));
        // accepts_task_fn(AsMethod(with_recv_and_str as fn(_, _) -> _));

        fn accepts_task_fn<F>(_task_fn: F)
        where
            F: TaskFn,
        {
        }

        accepts_task_fn(AsFunction(no_args as fn() -> _));
        accepts_task_fn(AsFunction(one_arg as fn(_) -> _));
        accepts_task_fn(AsMethod(with_recv as fn(_) -> _));
        accepts_task_fn(AsMethod(with_recv_and_str as fn(_, _) -> _));
        accepts_task_fn(AsAsyncMethod(async_with_recv_and_str as fn(_, _) -> _));
    }
}
