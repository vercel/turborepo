use anyhow::{anyhow, Context, Result};
use lazy_static::lazy_static;
use std::{future::Future, sync::Arc, time::Duration};
use turbo_tasks::{
    dynamic_call,
    macro_helpers::{new_node_intern, Node},
    NativeFunction, NodeReuseMode, NodeType,
};

// [turbo_function]
// pub async fn add(a: I32ValueRef, b: I32ValueRef) -> I32ValueRef {
//     let a = a.get().value;
//     let b = b.get().value;
//     println!("{} + {} = ...", a, b);
//     async_std::task::sleep(Duration::from_secs(1)).await;
//     println!("{} + {} = {}", a, b, a + b);
//     I32ValueRef::new(a + b)
// }

pub async fn add_impl(a: I32ValueRef, b: I32ValueRef) -> I32ValueRef {
    let a = a.get().value;
    let b = b.get().value;
    println!("{} + {} = ...", a, b);
    async_std::task::sleep(Duration::from_secs(1)).await;
    println!("{} + {} = {}", a, b, a + b);
    I32ValueRef::new(a + b)
}

// TODO autogenerate that
lazy_static! {
    static ref ADD_FUNCTION: NativeFunction = NativeFunction::new(|inputs| {
        let mut iter = inputs.into_iter();
        let a = iter
            .next()
            .ok_or_else(|| anyhow!("add() first argument missing"))?;
        let b = iter
            .next()
            .ok_or_else(|| anyhow!("add() second argument missing"))?;
        if iter.next().is_some() {
            return Err(anyhow!("add() called with too many arguments"));
        }
        I32ValueRef::verify(&a).context("add() invalid 1st argument")?;
        I32ValueRef::verify(&b).context("add() invalid 2nd argument")?;
        Ok(Box::new(move || {
            let a = a.clone();
            let b = b.clone();
            Box::pin(async move {
                let a = I32ValueRef::from_node(a).unwrap();
                let b = I32ValueRef::from_node(b).unwrap();
                add_impl(a, b).await.into()
            })
        }))
    });
}

pub fn add(a: I32ValueRef, b: I32ValueRef) -> impl Future<Output = I32ValueRef> {
    // TODO decide if we want to schedule or execute directly
    // directly would be `add_impl(a, b)`
    let result = dynamic_call(&ADD_FUNCTION, vec![a.into(), b.into()]).unwrap();
    return async { I32ValueRef::from_node(result.await).unwrap() };
}

// node! {
//   struct I32Value {
//     value: i32,
//   }
// }

// [turbo_node]
pub struct I32Value {
    pub value: i32,
}

impl I32Value {
    // [turbo_constructor(NodeReuseMode::GlobalInterning)]
    pub fn new(value: i32) -> Self {
        Self { value }
    }
}

// TODO autogenerate I32ValueRef
#[derive(Clone, Debug)]
pub struct I32ValueRef {
    node: Arc<Node>,
}

lazy_static! {
    static ref I32_VALUE_NODE_TYPE: NodeType =
        NodeType::new("I32Value".to_string(), NodeReuseMode::GlobalInterning);
}

impl I32ValueRef {
    pub fn new(value: i32) -> Self {
        let new_node = new_node_intern::<I32Value, _, _>(value, || {
            Arc::new(Node::new(
                &I32_VALUE_NODE_TYPE,
                Arc::new(I32Value::new(value)),
            ))
        });
        // let new_node = Arc::new(Node::new(
        //     &I32_VALUE_NODE_TYPE,
        //     Arc::new(I32Value::new(value)),
        // ));
        Self { node: new_node }
    }

    pub fn from_node(node: Arc<Node>) -> Option<Self> {
        if node.is_node_type(&I32_VALUE_NODE_TYPE) {
            Some(I32ValueRef { node })
        } else {
            None
        }
    }

    pub fn verify(node: &Arc<Node>) -> Result<()> {
        if node.is_node_type(&I32_VALUE_NODE_TYPE) {
            Ok(())
        } else {
            Err(anyhow!(
                "expected {:?} but got {:?}",
                *I32_VALUE_NODE_TYPE,
                node.get_node_type()
            ))
        }
    }

    pub fn get(&self) -> Arc<I32Value> {
        // unwrap is safe here since we ensure that it will be the correct node type
        self.node.read::<I32Value>().unwrap()
    }
}

impl From<I32ValueRef> for Arc<Node> {
    fn from(node_ref: I32ValueRef) -> Self {
        node_ref.node
    }
}
