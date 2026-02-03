use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{Config, Queue, QueueError, SendFunc, Stats, Wal};

struct Metrics {
    enqueued_total: AtomicU64,
    dequeued_total: AtomicU64,
    dropped_total: AtomicU64,
    overflow_count: AtomicU64,
    last_flush_millis: AtomicI64,
    started: AtomicBool,
    closed: AtomicBool,
    stop_flag: AtomicBool,
}

impl Metrics {
    fn new() -> Self {
        Self {
            enqueued_total: AtomicU64::new(0),
            dequeued_total: AtomicU64::new(0),
            dropped_total: AtomicU64::new(0),
            overflow_count: AtomicU64::new(0),
            last_flush_millis: AtomicI64::new(0),
            started: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            stop_flag: AtomicBool::new(false),
        }
    }
}

pub struct DiskQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    config: Config,
    send_func: SendFunc<T>,
    wal: Arc<Wal<T>>,
    wal_pending: Arc<AtomicUsize>,
    metrics: Arc<Metrics>,
    notify_tx: mpsc::Sender<()>,
    notify_rx: Mutex<Option<mpsc::Receiver<()>>>,
    processor: Mutex<Option<JoinHandle<()>>>,
}

impl<T> DiskQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    pub fn new(
        config: Config,
        send_func: impl Fn(Vec<T>) -> Result<(), QueueError> + Send + Sync + 'static,
    ) -> Result<Self, QueueError> {
        config.validate()?;
        let wal = Arc::new(Wal::open(&config)?);
        let (notify_tx, notify_rx) = mpsc::channel();
        let pending = wal.record_count()? as usize;
        Ok(Self {
            config,
            send_func: Arc::new(send_func),
            wal,
            wal_pending: Arc::new(AtomicUsize::new(pending)),
            metrics: Arc::new(Metrics::new()),
            notify_tx,
            notify_rx: Mutex::new(Some(notify_rx)),
            processor: Mutex::new(None),
        })
    }
}

impl<T> Queue<T> for DiskQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    fn start(&self) -> Result<(), QueueError> {
        if self.metrics.closed.load(Ordering::SeqCst) {
            return Err(QueueError::QueueClosed);
        }
        if self.metrics.started.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.metrics.stop_flag.store(false, Ordering::SeqCst);
        let notify_rx = self
            .notify_rx
            .lock()
            .map_err(|_| QueueError::QueueClosed)?
            .take();
        let notify_rx = notify_rx.ok_or(QueueError::QueueClosed)?;
        let config = self.config.clone();
        let send_func = self.send_func.clone();
        let wal = Arc::clone(&self.wal);
        let wal_pending = Arc::clone(&self.wal_pending);
        let metrics = Arc::clone(&self.metrics);
        let handle = thread::spawn(move || {
            run_processor(config, send_func, wal, wal_pending, metrics, notify_rx);
        });
        *self.processor.lock().map_err(|_| QueueError::QueueClosed)? = Some(handle);
        if self.wal_pending.load(Ordering::Relaxed) > 0 {
            let _ = self.notify_tx.send(());
        }
        Ok(())
    }

    fn stop(&self) -> Result<(), QueueError> {
        if self.metrics.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.metrics.stop_flag.store(true, Ordering::SeqCst);
        let _ = self.notify_tx.send(());
        if let Some(handle) = self.processor.lock().map_err(|_| QueueError::QueueClosed)?.take() {
            let _ = handle.join();
        }
        self.metrics.started.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn enqueue(&self, item: T) -> Result<(), QueueError> {
        if self.metrics.closed.load(Ordering::Acquire) {
            return Err(QueueError::QueueClosed);
        }
        self.wal.write(&item)?;
        self.metrics.enqueued_total.fetch_add(1, Ordering::Relaxed);
        self.wal_pending.fetch_add(1, Ordering::Relaxed);
        let _ = self.notify_tx.send(());
        Ok(())
    }

    fn get_stats(&self) -> Stats {
        let pending = self.wal_pending.load(Ordering::Relaxed);
        Stats {
            enqueued_total: self.metrics.enqueued_total.load(Ordering::Relaxed),
            dequeued_total: self.metrics.dequeued_total.load(Ordering::Relaxed),
            dropped_total: self.metrics.dropped_total.load(Ordering::Relaxed),
            overflow_count: self.metrics.overflow_count.load(Ordering::Relaxed),
            utilization_pct: 0.0,
            pending_count: pending,
            last_flush_time: millis_to_time(self.metrics.last_flush_millis.load(Ordering::Relaxed)),
            total_items: pending,
            memory_size: self.config.memory_size,
            local_buffers: 0,
            shard_count: 0,
            last_alert_time: None,
        }
    }

    fn is_started(&self) -> bool {
        self.metrics.started.load(Ordering::Relaxed)
    }

    fn is_closed(&self) -> bool {
        self.metrics.closed.load(Ordering::Relaxed)
    }
}

impl<T> Drop for DiskQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    fn drop(&mut self) {
        let _ = self.stop();
        let _ = self.wal.close();
    }
}

fn run_processor<T>(
    config: Config,
    send_func: SendFunc<T>,
    wal: Arc<Wal<T>>,
    wal_pending: Arc<AtomicUsize>,
    metrics: Arc<Metrics>,
    notify_rx: mpsc::Receiver<()>,
) where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    let mut batch_size = config.batch_size;
    let mut avg_latency_ns = 0u64;
    let mut last_flush = Instant::now();

    loop {
        if metrics.stop_flag.load(Ordering::Acquire) {
            drain_wal(&config, &send_func, &wal, &wal_pending, &metrics, &mut avg_latency_ns, &mut batch_size);
            break;
        }
        let timeout = config
            .flush_timeout
            .checked_sub(last_flush.elapsed())
            .unwrap_or(Duration::from_secs(0));
        let _ = notify_rx.recv_timeout(timeout);
        let timed_out = last_flush.elapsed() >= config.flush_timeout;
        if timed_out || wal_pending.load(Ordering::Relaxed) > 0 {
            drain_wal(&config, &send_func, &wal, &wal_pending, &metrics, &mut avg_latency_ns, &mut batch_size);
            last_flush = Instant::now();
        }
    }
}

fn drain_wal<T>(
    config: &Config,
    send_func: &SendFunc<T>,
    wal: &Wal<T>,
    wal_pending: &AtomicUsize,
    metrics: &Metrics,
    avg_latency_ns: &mut u64,
    batch_size: &mut usize,
) where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    let items = match wal.read_and_clear() {
        Ok(items) => items,
        Err(_) => return,
    };
    if items.is_empty() {
        wal_pending.store(0, Ordering::Relaxed);
        return;
    }
    wal_pending.store(0, Ordering::Relaxed);
    for chunk in items.chunks(*batch_size) {
        let start = Instant::now();
        if send_with_retry(send_func, config, chunk).is_ok() {
            metrics
                .dequeued_total
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
            metrics.last_flush_millis.store(now_millis(), Ordering::Relaxed);
            let latency_ns = start.elapsed().as_nanos() as u64;
            update_latency(latency_ns, avg_latency_ns);
            if config.adaptive_batch {
                *batch_size = adjust_batch(*batch_size, *avg_latency_ns, latency_ns, config);
            }
        } else {
            let _ = wal.write_batch(chunk);
            wal_pending.fetch_add(chunk.len(), Ordering::Relaxed);
        }
    }
}

fn send_with_retry<T>(send_func: &SendFunc<T>, config: &Config, batch: &[T]) -> Result<(), QueueError>
where
    T: Clone,
{
    let mut last_error = None;
    for attempt in 0..=config.retry_limit {
        if attempt > 0 {
            thread::sleep(Duration::from_millis((attempt as u64) * 100));
        }
        let start = Instant::now();
        let result = (send_func)(batch.to_vec());
        let elapsed = start.elapsed();
        if elapsed > config.send_timeout {
            last_error = Some(QueueError::SendFailed);
            continue;
        }
        match result {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.unwrap_or(QueueError::SendFailed))
}

fn adjust_batch(current: usize, avg_latency_ns: u64, last_latency_ns: u64, config: &Config) -> usize {
    if avg_latency_ns == 0 {
        return current;
    }
    let ratio = config.batch_adjust_ratio;
    let mut new_size = current as f64;
    if last_latency_ns < (avg_latency_ns as f64 * 0.9) as u64 {
        new_size += new_size * ratio;
    } else if last_latency_ns > (avg_latency_ns as f64 * 1.1) as u64 {
        new_size -= new_size * ratio;
    }
    let bounded = new_size.round() as usize;
    bounded.clamp(config.min_batch_size, config.max_batch_size)
}

fn update_latency(sample: u64, avg: &mut u64) {
    if *avg == 0 {
        *avg = sample;
    } else {
        *avg = (*avg * 9 + sample) / 10;
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as i64
}

fn millis_to_time(value: i64) -> Option<SystemTime> {
    if value <= 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_millis(value as u64))
}
