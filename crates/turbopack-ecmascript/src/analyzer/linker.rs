use std::{
    collections::{HashMap, HashSet},
    future::Future,
    mem::take,
    sync::Arc,
};

use anyhow::Result;
use swc_core::ecma::ast::Id;

use super::{graph::VarGraph, JsValue};

pub async fn link<'a, B, RB, F, RF>(
    graph: &VarGraph,
    mut val: JsValue,
    early_visitor: &B,
    visitor: &F,
    fun_args_values: HashMap<u32, Vec<JsValue>>,
) -> Result<JsValue>
where
    RB: 'a + Future<Output = Result<(JsValue, bool)>> + Send,
    B: 'a + Fn(JsValue) -> RB + Sync,
    RF: 'a + Future<Output = Result<(JsValue, bool)>> + Send,
    F: 'a + Fn(JsValue) -> RF + Sync,
{
    val.normalize();
    let val = link_internal_iterative(graph, val, early_visitor, visitor, fun_args_values).await?;
    Ok(val)
}

const LIMIT_NODE_SIZE: usize = 300;
const LIMIT_IN_PROGRESS_NODES: usize = 1000;
const LIMIT_LINK_STEPS: usize = 1500;

pub(crate) async fn link_internal_iterative<'a, B, RB, F, RF>(
    graph: &'a VarGraph,
    val: JsValue,
    early_visitor: &'a B,
    visitor: &'a F,
    mut fun_args_values: HashMap<u32, Vec<JsValue>>,
) -> Result<JsValue>
where
    RB: 'a + Future<Output = Result<(JsValue, bool)>> + Send,
    B: 'a + Fn(JsValue) -> RB + Sync,
    RF: 'a + Future<Output = Result<(JsValue, bool)>> + Send,
    F: 'a + Fn(JsValue) -> RF + Sync,
{
    #[derive(Debug)]
    enum Step {
        Enter(JsValue),
        EarlyVisit(JsValue),
        Leave(JsValue),
        LeaveVar(Id),
        LeaveLate(JsValue),
        Visit(JsValue),
        LeaveCall(u32),
    }

    let mut queue: Vec<Step> = Vec::new();
    let mut done: Vec<JsValue> = Vec::new();
    // Tracks the number of nodes in the queue and done combined
    let mut total_nodes = 0;
    let mut cycle_stack: HashSet<Id> = HashSet::new();
    // Tracks the number linking steps so far
    let mut steps = 0;

    total_nodes += val.total_nodes();
    queue.push(Step::Enter(val));

    while let Some(step) = queue.pop() {
        steps += 1;

        match step {
            // Enter a variable
            // - replace it with value from graph
            // - process value
            // - on leave: cache value
            Step::Enter(JsValue::Variable(var)) => {
                // Replace with unknown for now
                if cycle_stack.contains(&var) {
                    done.push(JsValue::Unknown(
                        Some(Arc::new(JsValue::Variable(var.clone()))),
                        "circular variable reference",
                    ));
                } else {
                    total_nodes -= 1;
                    if let Some(val) = graph.values.get(&var) {
                        cycle_stack.insert(var.clone());
                        queue.push(Step::LeaveVar(var));
                        total_nodes += val.total_nodes();
                        queue.push(Step::Enter(val.clone()));
                    } else {
                        total_nodes += 1;
                        done.push(JsValue::Unknown(
                            Some(Arc::new(JsValue::Variable(var.clone()))),
                            "no value of this variable analysed",
                        ));
                    };
                }
            }
            // Leave a variable
            Step::LeaveVar(var) => {
                let val = done.pop().unwrap();
                cycle_stack.remove(&var);
                done.push(val);
            }
            // Enter a function argument
            // We want to replace the argument with the value from the function call
            Step::Enter(JsValue::Argument(func_ident, index)) => {
                total_nodes -= 1;
                if let Some(args) = fun_args_values.get(&func_ident) {
                    if let Some(val) = args.get(index) {
                        total_nodes += val.total_nodes();
                        done.push(val.clone());
                    } else {
                        total_nodes += 1;
                        done.push(JsValue::Unknown(
                            None,
                            "unknown function argument (out of bounds)",
                        ));
                    }
                } else {
                    total_nodes += 1;
                    done.push(JsValue::Unknown(
                        Some(Arc::new(JsValue::Argument(func_ident, index))),
                        "function calls are not analysed yet",
                    ));
                }
            }
            // Visit a function call
            // This need special handling, since we want to replace the function call and process
            // the function return value after that.
            Step::Visit(JsValue::Call(
                _,
                box JsValue::Function(_, func_ident, return_value),
                args,
            )) => {
                total_nodes -= 2; // Call + Function
                if fun_args_values.contains_key(&func_ident) {
                    total_nodes -= return_value.total_nodes();
                    for arg in args.iter() {
                        total_nodes -= arg.total_nodes();
                    }
                    total_nodes += 1;
                    done.push(JsValue::Unknown(
                        Some(Arc::new(JsValue::call(
                            box JsValue::function(func_ident, return_value),
                            args,
                        ))),
                        "recursive function call",
                    ));
                } else {
                    // Return value will stay in total_nodes
                    for arg in args.iter() {
                        total_nodes -= arg.total_nodes();
                    }
                    fun_args_values.insert(func_ident, args);
                    queue.push(Step::LeaveCall(func_ident));
                    queue.push(Step::Enter(*return_value));
                }
            }
            // Leaving a function call evaluation
            // - remove function arguments from the map
            Step::LeaveCall(func_ident) => {
                fun_args_values.remove(&func_ident);
            }
            // Enter a function
            // We don't want to process the function return value yet, this will happen after
            // function calls
            // - just put it into done
            Step::Enter(func @ JsValue::Function(..)) => {
                done.push(func);
            }
            // Enter a value
            // - take and queue children for processing
            // - on leave: insert children again and optimize
            Step::Enter(mut val) => {
                let i = queue.len();
                queue.push(Step::Leave(JsValue::default()));
                let mut has_early_children = false;
                val.for_each_early_children_mut(true, &mut |child| {
                    has_early_children = true;
                    queue.push(Step::Enter(take(child)));
                    false
                });
                if has_early_children {
                    queue[i] = Step::EarlyVisit(val);
                } else {
                    val.for_each_children_mut(&mut |child| {
                        queue.push(Step::Enter(take(child)));
                        false
                    });
                    queue[i] = Step::Leave(val);
                }
            }
            // Early visit a value
            // - reconstruct the value from early children
            // - visit the value
            // - insert late children and process for Leave
            Step::EarlyVisit(mut val) => {
                val.for_each_early_children_mut(true, &mut |child| {
                    let val = done.pop().unwrap();
                    *child = val;
                    true
                });
                #[cfg(debug_assertions)]
                val.assert_total_nodes_up_to_date();
                total_nodes -= val.total_nodes();
                if val.total_nodes() > LIMIT_NODE_SIZE {
                    total_nodes += 1;
                    done.push(JsValue::Unknown(None, "node limit reached"));
                    continue;
                }

                let (mut val, visit_modified) = early_visitor(val).await?;
                #[cfg(debug_assertions)]
                val.assert_total_nodes_up_to_date();
                if visit_modified {
                    if val.total_nodes() > LIMIT_NODE_SIZE {
                        total_nodes += 1;
                        done.push(JsValue::Unknown(None, "node limit reached"));
                        continue;
                    }
                }

                let count = val.total_nodes();
                if total_nodes + count > LIMIT_IN_PROGRESS_NODES {
                    // There is always space for one more node since we just popped at least one
                    // count
                    total_nodes += 1;
                    done.push(JsValue::Unknown(None, "in progress nodes limit reached"));
                    continue;
                }
                total_nodes += count;

                let i = queue.len();
                queue.push(Step::LeaveLate(JsValue::default()));
                val.for_each_early_children_mut(false, &mut |child| {
                    queue.push(Step::Enter(take(child)));
                    false
                });
                queue[i] = Step::LeaveLate(val);
            }
            // Leave a value
            Step::Leave(mut val) => {
                val.for_each_children_mut(&mut |child| {
                    let val = done.pop().unwrap();
                    *child = val;
                    true
                });
                #[cfg(debug_assertions)]
                val.assert_total_nodes_up_to_date();

                total_nodes -= val.total_nodes();

                if val.total_nodes() > LIMIT_NODE_SIZE {
                    total_nodes += 1;
                    done.push(JsValue::Unknown(None, "node limit reached"));
                    continue;
                }
                val.normalize_shallow();

                #[cfg(debug_assertions)]
                val.assert_total_nodes_up_to_date();

                total_nodes += val.total_nodes();
                queue.push(Step::Visit(val));
            }
            // Leave a value from EarlyVisit
            Step::LeaveLate(mut val) => {
                val.for_each_early_children_mut(false, &mut |child| {
                    let val = done.pop().unwrap();
                    *child = val;
                    true
                });
                #[cfg(debug_assertions)]
                val.assert_total_nodes_up_to_date();

                total_nodes -= val.total_nodes();

                if val.total_nodes() > LIMIT_NODE_SIZE {
                    total_nodes += 1;
                    done.push(JsValue::Unknown(None, "node limit reached"));
                    continue;
                }
                val.normalize_shallow();

                #[cfg(debug_assertions)]
                val.assert_total_nodes_up_to_date();

                total_nodes += val.total_nodes();
                queue.push(Step::Visit(val));
            }
            // Visit a value with the visitor
            // - visited value is put into done
            Step::Visit(val) => {
                total_nodes -= val.total_nodes();

                let (mut val, visit_modified) = visitor(val).await?;
                if visit_modified {
                    val.normalize_shallow();
                    #[cfg(debug_assertions)]
                    val.assert_total_nodes_up_to_date();
                    if val.total_nodes() > LIMIT_NODE_SIZE {
                        total_nodes += 1;
                        done.push(JsValue::Unknown(None, "node limit reached"));
                        continue;
                    }
                }

                let count = val.total_nodes();
                if total_nodes + count > LIMIT_IN_PROGRESS_NODES {
                    // There is always space for one more node since we just popped at least one
                    // count
                    total_nodes += 1;
                    done.push(JsValue::Unknown(None, "in progress nodes limit reached"));
                    continue;
                }
                total_nodes += count;
                if visit_modified {
                    queue.push(Step::Enter(val));
                } else {
                    done.push(val);
                }
            }
        }
        if steps > LIMIT_LINK_STEPS {
            return Ok(JsValue::Unknown(
                None,
                "max number of linking steps reached",
            ));
        }
    }

    let final_value = done.pop().unwrap();

    debug_assert!(queue.is_empty());
    debug_assert_eq!(total_nodes, final_value.total_nodes());

    Ok(final_value)
}
