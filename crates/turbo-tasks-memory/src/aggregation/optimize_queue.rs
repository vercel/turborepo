use std::{
    hash::Hash,
    mem::{replace, swap, take},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use parking_lot::{Mutex, MutexGuard};

use super::{balance_edge, increase_aggregation_number, AggregationContext};

pub struct OptimizeQueue<I: Clone + Eq + Hash> {
    inner: Mutex<OptimizeQueueInner<I>>,
}

struct OptimizeQueueInner<I: Clone + Eq + Hash> {
    queue: Vec<OptimizeQueueItem<I>>,
    // Buffer for the queue, to avoid reallocation
    // might have capacity
    // usually empty, expect when taking over processing
    queue_buffer: Vec<OptimizeQueueItem<I>>,
    is_processing: Option<Arc<OptimizeQueueInProgress>>,
}

struct OptimizeQueueInProgress {
    mutex: Mutex<()>,
    signal: AtomicBool,
}

impl<I: Clone + Eq + Hash> OptimizeQueue<I> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(OptimizeQueueInner {
                queue: Vec::new(),
                queue_buffer: Vec::new(),
                is_processing: None,
            }),
        }
    }

    fn push(&self, item: OptimizeQueueItem<I>) {
        self.inner.lock().queue.push(item);
    }

    pub fn balance_edge(&self, upper_id: I, target_id: I) {
        self.push(OptimizeQueueItem::BalanceEdge {
            upper_id,
            target_id,
        });
    }

    pub fn process<C: AggregationContext<NodeRef = I>>(&self, ctx: &C) {
        self.process_internal(ctx, false);
    }

    pub fn force_process<C: AggregationContext<NodeRef = I>>(&self, ctx: &C) {
        self.process_internal(ctx, true);
    }

    fn process_internal<C: AggregationContext<NodeRef = I>>(&self, ctx: &C, force: bool) {
        let start = Instant::now();
        let mut queue;
        let guard: MutexGuard<'static, ()>;
        let in_progress;
        {
            let mut inner = self.inner.lock();
            let mut is_processing = None;
            if let Some(is_processing_ref) = &inner.is_processing {
                if force {
                    if is_processing_ref
                        .signal
                        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                    {
                        // We own the processing now, but we need to wait for the existing to
                        // finish.
                        is_processing = Some(is_processing_ref.clone());
                    } else {
                        // Some other force_process owns the processing.
                        let is_processing = is_processing_ref.clone();
                        drop(inner);
                        // Wait for it to finish
                        let _ = is_processing.mutex.lock();
                        return;
                    }
                } else {
                    // Already processing, since it was not forced we can just return
                    return;
                }
            }
            if is_processing.is_none() && inner.queue.is_empty() {
                // Empty queue, fast return
                return;
            }
            in_progress = Arc::new(OptimizeQueueInProgress {
                mutex: Mutex::new(()),
                signal: AtomicBool::new(false),
            });
            guard = unsafe { std::mem::transmute(in_progress.mutex.lock()) };
            inner.is_processing = Some(in_progress.clone());
            if let Some(is_processing) = is_processing {
                drop(inner);
                // Wait for the old processing to finish
                let _ = is_processing.mutex.lock();
                inner = self.inner.lock();
            }
            let queue_buffer = take(&mut inner.queue_buffer);
            if queue_buffer.is_empty() {
                queue = replace(&mut inner.queue, queue_buffer);
            } else {
                queue = queue_buffer;
            }
        }
        loop {
            while let Some(item) = queue.pop() {
                item.process(ctx);
                if !force && in_progress.signal.load(Ordering::Relaxed) {
                    // Some other thread wants to take over processing
                    let mut inner = self.inner.lock();
                    inner.queue_buffer = queue;
                    drop(inner);
                    drop(guard);
                    drop(in_progress);
                    let e = start.elapsed();
                    if e.as_millis() > 10 {
                        println!("Taking over processing after {:?}", e);
                    }
                    return;
                }
            }
            let mut inner = self.inner.lock();
            swap(&mut inner.queue, &mut queue);
            if queue.is_empty() {
                inner.is_processing = None;
                inner.queue_buffer = queue;
                drop(inner);
                drop(guard);
                drop(in_progress);
                let e = start.elapsed();
                if e.as_millis() > 10 {
                    println!("Processed in {:?}", e);
                }
                return;
            }
        }
    }
}

enum OptimizeQueueItem<I: Clone + Eq + Hash> {
    BalanceEdge { upper_id: I, target_id: I },
}

impl<I: Clone + Eq + Hash> OptimizeQueueItem<I> {
    fn process<C: AggregationContext<NodeRef = I>>(&self, ctx: &C) {
        match self {
            OptimizeQueueItem::BalanceEdge {
                upper_id,
                target_id,
            } => {
                balance_edge(ctx, upper_id, target_id);
            }
        }
    }
}
