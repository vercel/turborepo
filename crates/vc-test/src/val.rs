use anyhow::Result;
use turbo_tasks::{debug::ValueDebug, Completion, Vc};
use turbo_tasks_memory::MemoryBackend;

use crate::register;

#[turbo_tasks::value]
pub struct Val {
    val: i32,
}

#[turbo_tasks::value_impl]
impl Val {
    #[turbo_tasks::function]
    pub fn new(val: i32) -> Vc<Self> {
        Val { val }.cell()
    }

    #[turbo_tasks::function]
    pub fn negate(&self) -> Vc<Self> {
        Val { val: -self.val }.cell()
    }
}

#[turbo_tasks::value_trait]
pub trait Add {
    fn add(self: Vc<Self>, val: i32) -> Vc<Self>;
    fn test(self: Vc<Self>, val: i32) -> Vc<Self>;
}

#[turbo_tasks::value_trait]
pub trait Sub: Add {
    fn sub(self: Vc<Self>, val: i32) -> Vc<Self> {
        self.add(-val)
    }
}

#[turbo_tasks::value_impl]
impl Add for Val {
    #[turbo_tasks::function]
    async fn add(self: Vc<Self>, val: i32) -> Result<Vc<Self>> {
        let val = self.await?.val + val;
        Ok(Val { val }.cell())
    }
}

#[turbo_tasks::value_impl]
impl Sub for Val {}

pub async fn test() -> Result<()> {
    let val = Val::new(5);
    let val2 = val.add(8);
    let val3 = val2.sub(3);

    let val_add: Vc<&dyn Add> = val.upcast();
    let val_add_2 = val_add.add(8);

    // val.minus_one();
    // val_add.minus_one();

    eprintln!(
        "{:?} {:?} {:?}",
        val2.dbg().await?,
        val_add_2.dbg().await?,
        val3.dbg().await?
    );
    Ok(())
}

pub async fn run() -> Result<()> {
    register();

    let tt = turbo_tasks::TurboTasks::new(MemoryBackend::new(8 * 1024 * 1024 * 1024));

    turbo_tasks::run_once(tt, test()).await?;

    Ok(())
}
