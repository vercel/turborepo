use std::{collections::HashMap, hash::Hash};

use futures::{future::join_all, stream::FuturesUnordered, StreamExt};
use petgraph::{
    visit::{IntoNeighborsDirected, IntoNodeIdentifiers},
    Direction,
};
use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    task::JoinHandle,
};
use tracing::log::trace;

pub struct Walker<N, S> {
    marker: std::marker::PhantomData<S>,
    cancel: watch::Sender<bool>,
    node_events: Option<mpsc::Receiver<(N, oneshot::Sender<()>)>>,
    join_handles: FuturesUnordered<JoinHandle<()>>,
}

pub struct Start;
pub struct Walking;

pub type WalkMessage<N> = (N, oneshot::Sender<()>);

// These constraint might look very stiff, but since all of the petgraph graph
// types use integers as node ids and GraphBase already constraints these types
// to Copy + Eq so adding Hash + Send + 'static as bounds aren't unreasonable.
impl<N: Eq + Hash + Copy + Send + 'static> Walker<N, Start> {
    /// Create a new graph walker for a DAG that emits nodes only once all of
    /// their dependencies have been processed.
    /// The graph should not be modified after a walker is created as the nodes
    /// emitted might no longer exist or might miss newly added edges.
    pub fn new<G: IntoNodeIdentifiers<NodeId = N> + IntoNeighborsDirected>(graph: G) -> Self {
        let (cancel, cancel_rx) = watch::channel(false);
        let mut txs = HashMap::new();
        let mut rxs = HashMap::new();
        for node in graph.node_identifiers() {
            // Each node can finish at most once so we set the capacity to 1
            let (tx, rx) = broadcast::channel::<()>(1);
            txs.insert(node, tx);
            rxs.insert(node, rx);
        }
        // We will be emitting at most txs.len() nodes so emitting a node should never
        // block
        //
        // Always have at least 1 entry in buffer or this will panic
        let (node_tx, node_rx) = mpsc::channel(std::cmp::max(txs.len(), 1));
        let join_handles = FuturesUnordered::new();
        for node in graph.node_identifiers() {
            let tx = txs.remove(&node).expect("should have sender for all nodes");
            let mut cancel_rx = cancel_rx.clone();
            let node_tx = node_tx.clone();
            let mut deps_rx = graph
                .neighbors_directed(node, Direction::Outgoing)
                .map(|dep| {
                    rxs.get(&dep)
                        .expect("graph should have all nodes")
                        .resubscribe()
                })
                .collect::<Vec<_>>();

            join_handles.push(tokio::spawn(async move {
                let deps_fut = join_all(deps_rx.iter_mut().map(|rx| rx.recv()));

                tokio::select! {
                    // If both the cancel and dependencies are ready, we want to
                    // execute the cancel instead of sending an additional node.
                    biased;
                    _ = cancel_rx.changed() => {
                        // If this future resolves this means that either:
                        // - cancel was updated, this can only happen through
                        //   the cancel method which only sets it to true
                        // - the cancel sender was dropped which is also a sign that we should exit
                    }
                    results = deps_fut => {
                        for res in results {
                            match res {
                                // No errors from reading dependency channels
                                Ok(()) => (),
                                // A dependency finished without sending a finish
                                // Could happen if a cancel is sent and is racing with deps
                                // so we interpret this as a cancel.
                                Err(broadcast::error::RecvError::Closed) => {
                                    return;
                                }
                                // A dependency sent a finish signal more than once
                                // which shouldn't be possible.
                                // Since the message is always the unit type we can proceed
                                // but we log as this is unexpected behavior.
                                Err(broadcast::error::RecvError::Lagged(x)) => {
                                    debug_assert!(false, "A node finished {x} more times than expected");
                                    trace!("A node finished {x} more times than expected");
                                }
                            }
                        }

                        let (callback_tx, callback_rx) = oneshot::channel::<()>();
                        // do some err handling with the send failure?
                        if node_tx.send((node, callback_tx)).await.is_err() {
                            // Receiving end of node channel has been closed/dropped
                            // Since there's nothing the mark the node as being done
                            // we act as if we have been canceled.
                            trace!("Receiver was dropped before walk finished without calling cancel");
                            return;
                        }
                        if callback_rx.await.is_err() {
                            // If the caller drops the callback sender without signaling
                            // that the node processing is finished we assume that it is finished.
                            trace!("Callback sender was dropped without sending a finish signal")
                        }
                        // Send errors indicate that there are no receivers which
                        // happens when this node has no dependents
                        tx.send(()).ok();
                    }
                }
            }));
        }

        // All senders should have been moved into their correct node task
        debug_assert!(txs.is_empty());

        Self {
            cancel,
            node_events: Some(node_rx),
            join_handles,
            marker: std::marker::PhantomData,
        }
    }

    /// Start walking the graph and return a channel that emits node
    /// indices once all of the nodes dependencies have finished.
    /// It is up to the caller to use the oneshot channel to mark
    /// a node as being done.
    pub fn walk(self) -> (Walker<N, Walking>, mpsc::Receiver<WalkMessage<N>>) {
        let Self {
            cancel,
            mut node_events,
            join_handles,
            ..
        } = self;
        let node_events = node_events
            .take()
            .expect("walking graph with walker that has already been used");
        (
            Walker {
                marker: std::marker::PhantomData,
                cancel,
                node_events: None,
                join_handles,
            },
            node_events,
        )
    }
}

impl<N> Walker<N, Walking> {
    /// Cancel the walk
    /// Any nodes that are already in the queue to be emitted will still be
    /// sent, but no new nodes will be sent.
    pub fn cancel(&mut self) -> Result<(), watch::error::SendError<bool>> {
        self.cancel.send(true)
    }

    /// Consumes the watcher and waits for all futures to be finished.
    /// Primarily used after the walk is canceled to ensure all tasks
    /// have been stopped.
    pub async fn wait(self) -> Result<(), tokio::task::JoinError> {
        let Self {
            mut join_handles, ..
        } = self;
        // Wait for all of the handles to finish up
        while let Some(result) = join_handles.next().await {
            result?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use petgraph::Graph;

    use super::*;

    #[tokio::test]
    async fn test_ordering() {
        // a -> b -> c
        let mut g = Graph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");
        g.add_edge(a, b, ());
        g.add_edge(b, c, ());

        let walker = Walker::new(&g);
        let mut visited = Vec::new();
        let (walker, mut node_emitter) = walker.walk();
        while let Some((index, done)) = node_emitter.recv().await {
            visited.push(index);
            done.send(()).unwrap();
        }
        walker.wait().await.unwrap();
        assert_eq!(visited, vec![c, b, a]);
    }

    #[tokio::test]
    async fn test_cancel() {
        // a -> b -> c
        let mut g = Graph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");
        g.add_edge(a, b, ());
        g.add_edge(b, c, ());

        let walker = Walker::new(&g);
        let mut visited = Vec::new();
        let (mut walker, mut node_emitter) = walker.walk();
        while let Some((index, done)) = node_emitter.recv().await {
            // Cancel after we get the first node
            walker.cancel().unwrap();

            visited.push(index);
            done.send(()).unwrap();
        }
        assert_eq!(visited, vec![c]);
        let Walker { join_handles, .. } = walker;
        // Yield to make sure the tasks have a chance to poll the cancel future
        tokio::time::sleep(Duration::from_millis(1)).await;

        // Verify that all node tasks stop running
        for join_handle in join_handles {
            assert!(join_handle.is_finished());
        }
    }
    // test that long running nodes block dependents, but others can continue
    #[tokio::test]
    async fn test_dependencies_block_ancestors() {
        // a -- b -- c
        //   \
        //    - d -- e
        let mut g = Graph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");
        let d = g.add_node("d");
        let e = g.add_node("e");
        g.add_edge(a, b, ());
        g.add_edge(a, d, ());
        g.add_edge(b, c, ());
        g.add_edge(d, e, ());

        // We intentionally wait to mark e as finished until b has been finished
        let walker = Walker::new(&g);
        let visited = Arc::new(Mutex::new(Vec::new()));
        let (walker, mut node_emitter) = walker.walk();
        let (b_done, is_b_done) = oneshot::channel::<()>();
        let mut b_done = Some(b_done);
        let mut is_b_done = Some(is_b_done);
        while let Some((index, done)) = node_emitter.recv().await {
            if index == e {
                // don't mark as done until we get the signal that b is finished
                let is_b_done = is_b_done.take().unwrap();
                let visited = visited.clone();
                tokio::spawn(async move {
                    is_b_done.await.unwrap();
                    visited.lock().unwrap().push(index);
                    done.send(()).unwrap();
                });
            } else if index == b {
                // send the signal that b is finished
                visited.lock().unwrap().push(index);
                done.send(()).unwrap();
                b_done.take().unwrap().send(()).unwrap();
            } else {
                visited.lock().unwrap().push(index);
                done.send(()).unwrap();
            }
        }
        walker.wait().await.unwrap();
        assert_eq!(visited.lock().unwrap().as_slice(), &[c, b, e, d, a]);
    }

    #[tokio::test]
    async fn test_multiple_roots() {
        // a -- b -- c
        //          /
        // d -- e -
        let mut g = Graph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");
        let d = g.add_node("d");
        let e = g.add_node("e");
        g.add_edge(a, b, ());
        g.add_edge(b, c, ());
        g.add_edge(d, e, ());
        g.add_edge(e, c, ());

        // We intentionally wait to mark e as finished until b has been finished
        let walker = Walker::new(&g);
        let visited = Arc::new(Mutex::new(Vec::new()));
        let (walker, mut node_emitter) = walker.walk();
        let (b_done, is_b_done) = oneshot::channel::<()>();
        let (d_done, is_d_done) = oneshot::channel::<()>();
        let mut b_done = Some(b_done);
        let mut is_b_done = Some(is_b_done);
        let mut d_done = Some(d_done);
        let mut is_d_done = Some(is_d_done);
        while let Some((index, done)) = node_emitter.recv().await {
            if index == e {
                // don't mark as done until we get the signal that b is finished
                let is_b_done = is_b_done.take().unwrap();
                let visited = visited.clone();
                tokio::spawn(async move {
                    is_b_done.await.unwrap();
                    visited.lock().unwrap().push(index);
                    done.send(()).unwrap();
                });
            } else if index == b {
                // send the signal that b is finished
                visited.lock().unwrap().push(index);
                done.send(()).unwrap();
                b_done.take().unwrap().send(()).unwrap();
            } else if index == a {
                // don't mark as done until d finishes
                let is_d_done = is_d_done.take().unwrap();
                let visited = visited.clone();
                tokio::spawn(async move {
                    is_d_done.await.unwrap();
                    visited.lock().unwrap().push(index);
                    done.send(()).unwrap();
                });
            } else if index == d {
                // send the signal that b is finished
                visited.lock().unwrap().push(index);
                done.send(()).unwrap();
                d_done.take().unwrap().send(()).unwrap();
            } else {
                visited.lock().unwrap().push(index);
                done.send(()).unwrap();
            }
        }
        walker.wait().await.unwrap();
        assert_eq!(visited.lock().unwrap().as_slice(), &[c, b, e, d, a]);
    }
}
