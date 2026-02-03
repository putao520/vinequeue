use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{Config, QueueError, SendFunc, Stats, Wal};

use super::local_buffer::LocalBuffers;
use super::processor::{millis_to_time, now_millis, run_processor};
use super::shared::{CentralShard, Metrics};

static QUEUE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
pub(crate) enum OverflowPolicy {
    Reject,
    Wal,
}

pub(crate) struct CoreQueue<T> {
    config: Config,
    send_func: SendFunc<T>,
    shards: Arc<Vec<CentralShard<T>>>,
    shard_mask: u64,
    local_buffers: Arc<LocalBuffers<T>>,
    metrics: Arc<Metrics>,
    wal: Option<Arc<Wal<T>>>,
    wal_pending: Arc<AtomicUsize>,
    overflow_policy: OverflowPolicy,
    notify_tx: mpsc::Sender<()>,
    notify_rx: Mutex<Option<mpsc::Receiver<()>>>,
    processor: Mutex<Option<thread::JoinHandle<()>>>,
}

impl<T> CoreQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    pub(crate) fn new(
        config: Config,
        send_func: SendFunc<T>,
        wal: Option<Arc<Wal<T>>>,
        overflow: OverflowPolicy,
    ) -> Result<Self, QueueError> {
        config.validate()?;
        let queue_id = QUEUE_ID.fetch_add(1, Ordering::Relaxed);
        let local_buffer_size = config.batch_size.max(1);
        let local_buffer_count = (config.memory_size / local_buffer_size).max(1);
        let parallel = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let shard_count = parallel.next_power_of_two().max(1);
        let shard_mask = (shard_count - 1) as u64;

        let mut shards = Vec::with_capacity(shard_count);
        for _ in 0..shard_count {
            shards.push(CentralShard::new());
        }
        let (notify_tx, notify_rx) = mpsc::channel();

        Ok(Self {
            config,
            send_func,
            shards: Arc::new(shards),
            shard_mask,
            local_buffers: Arc::new(LocalBuffers::new(queue_id, local_buffer_count, local_buffer_size)),
            metrics: Arc::new(Metrics::new()),
            wal,
            wal_pending: Arc::new(AtomicUsize::new(0)),
            overflow_policy: overflow,
            notify_tx,
            notify_rx: Mutex::new(Some(notify_rx)),
            processor: Mutex::new(None),
        })
    }

    pub(crate) fn start(&self) -> Result<(), QueueError> {
        if self.metrics.closed.load(Ordering::SeqCst) {
            return Err(QueueError::QueueClosed);
        }
        if self.metrics.started.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.metrics.stop_flag.store(false, Ordering::SeqCst);
        if let Some(wal) = self.wal.as_ref() {
            let recovered = wal.read_all()?;
            if !recovered.is_empty() {
                let recovered_len = recovered.len();
                for (idx, item) in recovered.into_iter().enumerate() {
                    let shard_index = idx % self.shards.len();
                    let mut guard = self.shards[shard_index]
                        .items
                        .lock()
                        .map_err(|_| QueueError::QueueClosed)?;
                    guard.push(item);
                }
                self.metrics
                    .total_in_memory
                    .fetch_add(recovered_len, Ordering::Relaxed);
                wal.clear()?;
                self.wal_pending.store(0, Ordering::Relaxed);
                let _ = self.notify_tx.send(());
            }
        }
        let notify_rx = self
            .notify_rx
            .lock()
            .map_err(|_| QueueError::QueueClosed)?
            .take();
        let notify_rx = notify_rx.ok_or(QueueError::QueueClosed)?;
        let shards = Arc::clone(&self.shards);
        let local_buffers = Arc::clone(&self.local_buffers);
        let metrics = Arc::clone(&self.metrics);
        let wal = self.wal.clone();
        let wal_pending = Arc::clone(&self.wal_pending);
        let config = self.config.clone();
        let send_func = self.send_func.clone();
        let shard_mask = self.shard_mask;
        let handle = thread::spawn(move || {
            run_processor(
                config,
                send_func,
                shards,
                local_buffers,
                metrics,
                wal,
                wal_pending,
                notify_rx,
                shard_mask,
            );
        });
        *self.processor.lock().map_err(|_| QueueError::QueueClosed)? = Some(handle);
        Ok(())
    }

    pub(crate) fn stop(&self) -> Result<(), QueueError> {
        if self.metrics.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let mut flush_error = None;
        self.local_buffers.flush_all(|items| {
            if flush_error.is_some() {
                return;
            }
            if let Err(err) = self.flush_to_central(items) {
                flush_error = Some(err);
            }
        });
        if let Some(err) = flush_error {
            return Err(err);
        }
        self.metrics.stop_flag.store(true, Ordering::SeqCst);
        let _ = self.notify_tx.send(());
        if let Some(handle) = self.processor.lock().map_err(|_| QueueError::QueueClosed)?.take() {
            let _ = handle.join();
        }
        self.metrics.started.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub(crate) fn enqueue(&self, item: T) -> Result<(), QueueError> {
        if self.metrics.closed.load(Ordering::Acquire) {
            return Err(QueueError::QueueClosed);
        }
        let hard_limit = self.config.memory_size * self.config.hard_limit_pct as usize / 100;
        let soft_limit = self.config.memory_size * self.config.soft_limit_pct as usize / 100;

        let new_total = self.metrics.total_in_memory.fetch_add(1, Ordering::Relaxed) + 1;
        if new_total > hard_limit {
            self.metrics.total_in_memory.fetch_sub(1, Ordering::Relaxed);
            self.metrics.overflow_count.fetch_add(1, Ordering::Relaxed);
            return self.handle_hard_limit(item);
        }
        if new_total >= soft_limit {
            self.metrics.overflow_count.fetch_add(1, Ordering::Relaxed);
            self.metrics.last_alert_millis.store(now_millis(), Ordering::Relaxed);
            thread::sleep(self.config.backpressure_delay);
        }

        let mut item_opt = Some(item);
        if let Some(index) = self.local_buffers.local_index() {
            enum BufferAction<T> {
                Pushed,
                Flush(Vec<T>),
            }
            let action = self.local_buffers.with_buffer(index, |buffer| {
                let value = item_opt.take().expect("item missing");
                if buffer.len() < buffer.capacity() {
                    buffer.push(value);
                    BufferAction::Pushed
                } else {
                    let mut drained = Vec::with_capacity(buffer.capacity() + 1);
                    buffer.drain_into(&mut drained);
                    drained.push(value);
                    BufferAction::Flush(drained)
                }
            });
            if let Some(action) = action {
                match action {
                    BufferAction::Pushed => {}
                    BufferAction::Flush(drained) => {
                        self.flush_to_central(drained)?;
                    }
                }
                self.metrics.enqueued_total.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            if let Some(value) = item_opt.take() {
                self.flush_to_central(vec![value])?;
                self.metrics.enqueued_total.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        }
        if let Some(value) = item_opt.take() {
            self.flush_to_central(vec![value])?;
        }
        self.metrics.enqueued_total.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub(crate) fn stats(&self) -> Stats {
        let total_in_memory = self.metrics.total_in_memory.load(Ordering::Relaxed);
        let wal_pending = self.wal_pending.load(Ordering::Relaxed);
        let utilization_pct = if self.config.memory_size == 0 {
            0.0
        } else {
            (total_in_memory as f32 / self.config.memory_size as f32) * 100.0
        };
        Stats {
            enqueued_total: self.metrics.enqueued_total.load(Ordering::Relaxed),
            dequeued_total: self.metrics.dequeued_total.load(Ordering::Relaxed),
            dropped_total: self.metrics.dropped_total.load(Ordering::Relaxed),
            overflow_count: self.metrics.overflow_count.load(Ordering::Relaxed),
            utilization_pct,
            pending_count: total_in_memory + wal_pending,
            last_flush_time: millis_to_time(self.metrics.last_flush_millis.load(Ordering::Relaxed)),
            total_items: total_in_memory + wal_pending,
            memory_size: self.config.memory_size,
            local_buffers: self.local_buffers.count(),
            shard_count: self.shards.len(),
            last_alert_time: millis_to_time(self.metrics.last_alert_millis.load(Ordering::Relaxed)),
        }
    }

    pub(crate) fn is_started(&self) -> bool {
        self.metrics.started.load(Ordering::Relaxed)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.metrics.closed.load(Ordering::Relaxed)
    }

    fn flush_to_central(&self, items: Vec<T>) -> Result<(), QueueError> {
        let mut hasher = DefaultHasher::new();
        thread::current().id().hash(&mut hasher);
        let shard_index = (hasher.finish() & self.shard_mask) as usize;
        let shard = &self.shards[shard_index];
        let mut guard = shard.items.lock().map_err(|_| QueueError::QueueClosed)?;
        guard.extend(items);
        let _ = self.notify_tx.send(());
        Ok(())
    }

    fn handle_hard_limit(&self, item: T) -> Result<(), QueueError> {
        match self.overflow_policy {
            OverflowPolicy::Reject => Err(QueueError::BufferFull),
            OverflowPolicy::Wal => {
                let wal = self.wal.as_ref().ok_or(QueueError::WalWriteFailed)?;
                wal.write(&item)?;
                self.wal_pending.fetch_add(1, Ordering::Relaxed);
                self.metrics.enqueued_total.fetch_add(1, Ordering::Relaxed);
                let _ = self.notify_tx.send(());
                Ok(())
            }
        }
    }
}
