use std::marker::PhantomData;

use anyhow::{bail, Result};

use crate::{ConcreteTaskInput, RawVc, Vc};

pub trait TaskInput: Send + Sync + Clone {
    fn try_from_concrete(input: &ConcreteTaskInput) -> Result<Self>;
    fn into_concrete(self) -> ConcreteTaskInput;
}

impl TaskInput for String {
    fn try_from_concrete(input: &ConcreteTaskInput) -> Result<Self> {
        match input {
            ConcreteTaskInput::String(s) => Ok(s.clone()),
            _ => bail!("invalid task input type, expected String"),
        }
    }

    fn into_concrete(self) -> ConcreteTaskInput {
        ConcreteTaskInput::String(self)
    }
}

impl TaskInput for bool {
    fn try_from_concrete(input: &ConcreteTaskInput) -> Result<Self> {
        match input {
            ConcreteTaskInput::Bool(b) => Ok(*b),
            _ => bail!("invalid task input type, expected Bool"),
        }
    }

    fn into_concrete(self) -> ConcreteTaskInput {
        ConcreteTaskInput::Bool(self)
    }
}

impl<'a, T> TaskInput for Vec<T>
where
    T: TaskInput,
{
    fn try_from_concrete(value: &ConcreteTaskInput) -> Result<Self> {
        match value {
            ConcreteTaskInput::List(list) => Ok(list
                .iter()
                .map(|i| <T as TaskInput>::try_from_concrete(i))
                .collect::<Result<Vec<_>, _>>()?),
            _ => bail!("invalid task input type, expected List"),
        }
    }

    fn into_concrete(self) -> ConcreteTaskInput {
        ConcreteTaskInput::List(
            self.into_iter()
                .map(|i| <T as TaskInput>::into_concrete(i))
                .collect::<Vec<_>>(),
        )
    }
}

impl TaskInput for u32 {
    fn try_from_concrete(value: &ConcreteTaskInput) -> Result<Self> {
        match value {
            ConcreteTaskInput::U32(value) => Ok(*value),
            _ => bail!("invalid task input type, expected U32"),
        }
    }

    fn into_concrete(self) -> ConcreteTaskInput {
        ConcreteTaskInput::U32(self)
    }
}

impl TaskInput for i32 {
    fn try_from_concrete(value: &ConcreteTaskInput) -> Result<Self> {
        match value {
            ConcreteTaskInput::I32(value) => Ok(*value),
            _ => bail!("invalid task input type, expected I32"),
        }
    }

    fn into_concrete(self) -> ConcreteTaskInput {
        ConcreteTaskInput::I32(self)
    }
}

impl TaskInput for u64 {
    fn try_from_concrete(value: &ConcreteTaskInput) -> Result<Self> {
        match value {
            ConcreteTaskInput::U64(value) => Ok(*value),
            _ => bail!("invalid task input type, expected U64"),
        }
    }

    fn into_concrete(self) -> ConcreteTaskInput {
        ConcreteTaskInput::U64(self)
    }
}

impl TaskInput for usize {
    fn try_from_concrete(value: &ConcreteTaskInput) -> Result<Self> {
        match value {
            ConcreteTaskInput::Usize(value) => Ok(*value),
            _ => bail!("invalid task input type, expected Usize"),
        }
    }

    fn into_concrete(self) -> ConcreteTaskInput {
        ConcreteTaskInput::Usize(self)
    }
}

impl<T> TaskInput for Option<T>
where
    T: TaskInput,
{
    fn try_from_concrete(value: &ConcreteTaskInput) -> Result<Self> {
        match value {
            ConcreteTaskInput::Nothing => Ok(None),
            _ => Ok(Some(<T as TaskInput>::try_from_concrete(value)?)),
        }
    }

    fn into_concrete(self) -> ConcreteTaskInput {
        match self {
            None => ConcreteTaskInput::Nothing,
            Some(value) => <T as TaskInput>::into_concrete(value),
        }
    }
}

impl<T> TaskInput for Vc<T> {
    fn try_from_concrete(input: &ConcreteTaskInput) -> Result<Self> {
        match input {
            ConcreteTaskInput::TaskCell(task, index) => Ok(Vc {
                node: RawVc::TaskCell(*task, *index),
                _t: PhantomData,
            }),
            ConcreteTaskInput::TaskOutput(task) => Ok(Vc {
                node: RawVc::TaskOutput(*task),
                _t: PhantomData,
            }),
            _ => bail!("invalid task input type, expected RawVc"),
        }
    }

    fn into_concrete(self) -> ConcreteTaskInput {
        match self.node {
            RawVc::TaskCell(task, index) => ConcreteTaskInput::TaskCell(task, index),
            RawVc::TaskOutput(task) => ConcreteTaskInput::TaskOutput(task),
        }
    }
}
