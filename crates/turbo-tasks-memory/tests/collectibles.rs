#![feature(arbitrary_self_types)]
#![feature(async_fn_in_trait)]

use std::collections::HashSet;

use anyhow::{bail, Result};
use tokio::time::sleep;
use turbo_tasks::{
    emit, primitives::StringVc, CollectiblesSource, NothingVc, ValueToString, ValueToStringVc,
};
use turbo_tasks_testing::{register, run};
register!();

#[tokio::test]
async fn transitive_emitting() {
    run! {
        let result = my_transitive_emitting_function("".to_string(), "".to_string());
        let list = result.peek_collectibles::<Box<dyn ValueToString>>().await?;
        assert_eq!(list.len(), 2);
        let mut expected = ["123", "42"].into_iter().collect::<HashSet<_>>();
        for collectible in list {
            assert!(expected.remove(collectible.to_string().await?.as_str()))
        }
        assert_eq!(result.await?.0, 0);
    }
}

#[tokio::test]
async fn multi_emitting() {
    run! {
        let result = my_multi_emitting_function();
        let list = result.peek_collectibles::<Box<dyn ValueToString>>().await?;
        assert_eq!(list.len(), 2);
        let mut expected = ["123", "42"].into_iter().collect::<HashSet<_>>();
        for collectible in list {
            assert!(expected.remove(collectible.to_string().await?.as_str()))
        }
        assert_eq!(result.await?.0, 0);
    }
}

#[tokio::test]
async fn taking_collectibles() {
    run! {
        let result = my_collecting_function();
        let list = result.take_collectibles::<Box<dyn ValueToString>>().await?;
        // my_collecting_function already processed the collectibles so the list should
        // be empty
        assert!(list.is_empty());
        assert_eq!(result.await?.0, 0);
    }
}

#[tokio::test]
async fn taking_collectibles_extra_layer() {
    run! {
        let result = my_collecting_function_indirect();
        let list = result.take_collectibles::<Box<dyn ValueToString>>().await?;
        // my_collecting_function already processed the collectibles so the list should
        // be empty
        assert!(list.is_empty());
        assert_eq!(result.await?.0, 0);
    }
}

#[tokio::test]
async fn taking_collectibles_parallel() {
    run! {
        let result = my_transitive_emitting_function("".to_string(), "a".to_string());
        let list = result.take_collectibles::<Box<dyn ValueToString>>().await?;
        assert_eq!(list.len(), 2);
        assert_eq!(result.await?.0, 0);

        let result = my_transitive_emitting_function("".to_string(), "b".to_string());
        let list = result.take_collectibles::<Box<dyn ValueToString>>().await?;
        assert_eq!(list.len(), 2);
        assert_eq!(result.await?.0, 0);

        let result = my_transitive_emitting_function_with_child_scope("".to_string(), "b".to_string(), "1".to_string());
        let list = result.take_collectibles::<Box<dyn ValueToString>>().await?;
        assert_eq!(list.len(), 2);
        assert_eq!(result.await?.0, 0);

        let result = my_transitive_emitting_function_with_child_scope("".to_string(), "b".to_string(), "2".to_string());
        let list = result.take_collectibles::<Box<dyn ValueToString>>().await?;
        assert_eq!(list.len(), 2);
        assert_eq!(result.await?.0, 0);

        let result = my_transitive_emitting_function_with_child_scope("".to_string(), "c".to_string(), "3".to_string());
        let list = result.take_collectibles::<Box<dyn ValueToString>>().await?;
        assert_eq!(list.len(), 2);
        assert_eq!(result.await?.0, 0);
    }
}

#[turbo_tasks::function]
async fn my_collecting_function() -> Result<Vc<Thing>> {
    let result = my_transitive_emitting_function("".to_string(), "".to_string());
    result.take_collectibles::<Box<dyn ValueToString>>().await?;
    Ok(result)
}

#[turbo_tasks::function]
async fn my_collecting_function_indirect() -> Result<ThingVc> {
    let result = my_collecting_function();
    let list = result.peek_collectibles::<Box<dyn ValueToString>>().await?;
    // my_collecting_function already processed the collectibles so the list should
    // be empty
    assert!(list.is_empty());
    Ok(result)
}

#[turbo_tasks::function]
fn my_multi_emitting_function() -> Vc<Thing> {
    my_transitive_emitting_function("".to_string(), "a".to_string());
    my_transitive_emitting_function("".to_string(), "b".to_string());
    my_emitting_function("".to_string());
    Thing::cell(Thing(0))
}

#[turbo_tasks::function]
fn my_transitive_emitting_function(key: String, _key2: String) -> Vc<Thing> {
    my_emitting_function(key);
    Thing::cell(Thing(0))
}

#[turbo_tasks::function]
async fn my_transitive_emitting_function_with_child_scope(
    key: &str,
    key2: &str,
    _key3: &str,
) -> Result<ThingVc> {
    let thing = my_transitive_emitting_function(key, key2);
    let list = thing.peek_collectibles::<Box<dyn ValueToString>>().await?;
    assert_eq!(list.len(), 2);
    Ok(thing)
}

#[turbo_tasks::function]
async fn my_emitting_function(_key: &str) -> Result<()> {
    sleep(Duration::from_millis(100)).await;
    emit(Vc::upcast::<Box<dyn ValueToString>>(Thing::new(123)));
    emit(Vc::upcast::<Box<dyn ValueToString>>(Thing::new(42)));
    Ok(())
}

#[turbo_tasks::value(shared)]
struct Thing(u32);

impl ThingVc {
    fn new(v: u32) -> Self {
        Self::cell(Thing(v))
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for Thing {
    #[turbo_tasks::function]
    fn to_string(&self) -> StringVc {
        StringVc::cell(self.0.to_string())
    }
}
