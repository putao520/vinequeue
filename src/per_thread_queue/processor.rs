use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{thread};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{Config, QueueError, SendFunc, Wal};

use super::local_buffer::LocalBuffers;
use super::shared::{CentralShard, Metrics};

pub(crate) fn run_processor<T>(
    config: Config,
    send_func: SendFunc<T>,
    shards: Arc<Vec<CentralShard<T>>>,
    local_buffers: Arc<LocalBuffers<T>>,
    metrics: Arc<Metrics>,
    wal: Option<Arc<Wal<T>>>,
    wal_pending: Arc<AtomicUsize>,
    notify_rx: mpsc::Receiver<()>,
    shard_mask: u64,
) where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    let mut batch_size = config.batch_size;
    let mut avg_latency_ns = 0u64;
    let mut last_flush = Instant::now();
    let mut shard_cursor = 0usize;

    loop {
        if metrics.stop_flag.load(Ordering::Acquire) {
            drain_all(
                &config,
                &send_func,
                &shards,
                &metrics,
                &wal,
                &wal_pending,
                &mut shard_cursor,
                shard_mask,
                batch_size,
            );
            break;
        }
        let timeout = config
            .flush_timeout
            .checked_sub(last_flush.elapsed())
            .unwrap_or(Duration::from_secs(0));
        let _ = notify_rx.recv_timeout(timeout);
        let timed_out = last_flush.elapsed() >= config.flush_timeout;

        if timed_out {
            flush_local_buffers(&local_buffers, &shards, &mut shard_cursor, shard_mask);
        }
        let mut batch = collect_batch(&shards, &metrics, batch_size, &mut shard_cursor, shard_mask);
        if timed_out && batch.is_empty() {
            batch = collect_batch(&shards, &metrics, batch_size, &mut shard_cursor, shard_mask);
        }
        if batch.is_empty() {
            if timed_out {
                last_flush = Instant::now();
            }
            if wal_pending.load(Ordering::Relaxed) > 0 {
                let _ = process_wal(
                    &config,
                    &send_func,
                    &wal,
                    &wal_pending,
                    &metrics,
                    &mut avg_latency_ns,
                    &mut batch_size,
                );
            }
            continue;
        }

        if timed_out || batch.len() >= batch_size {
            let start = Instant::now();
            if send_with_retry(&send_func, &config, &batch).is_ok() {
                metrics
                    .dequeued_total
                    .fetch_add(batch.len() as u64, Ordering::Relaxed);
                metrics.last_flush_millis.store(now_millis(), Ordering::Relaxed);
                let latency_ns = start.elapsed().as_nanos() as u64;
                update_latency(latency_ns, &mut avg_latency_ns);
                if config.adaptive_batch {
                    batch_size = adjust_batch(batch_size, avg_latency_ns, latency_ns, &config);
                }
            } else if let Some(wal) = wal.as_ref() {
                let _ = wal.write_batch(&batch);
                wal_pending.fetch_add(batch.len(), Ordering::Relaxed);
            } else {
                metrics
                    .dropped_total
                    .fetch_add(batch.len() as u64, Ordering::Relaxed);
            }
            last_flush = Instant::now();
        }
    }
}

fn collect_batch<T>(
    shards: &[CentralShard<T>],
    metrics: &Metrics,
    max_items: usize,
    cursor: &mut usize,
    shard_mask: u64,
) -> Vec<T> {
    let mut batch = Vec::with_capacity(max_items);
    for _ in 0..shards.len() {
        if batch.len() >= max_items {
            break;
        }
        let shard_index = *cursor & shard_mask as usize;
        *cursor = cursor.wrapping_add(1);
        let shard = &shards[shard_index];
        if let Ok(mut guard) = shard.items.lock() {
            if guard.is_empty() {
                continue;
            }
            let take = (max_items - batch.len()).min(guard.len());
            batch.extend(guard.drain(0..take));
            metrics.total_in_memory.fetch_sub(take, Ordering::Relaxed);
        }
    }
    batch
}

fn flush_local_buffers<T>(
    local_buffers: &LocalBuffers<T>,
    shards: &[CentralShard<T>],
    cursor: &mut usize,
    shard_mask: u64,
) {
    local_buffers.flush_all(|items| {
        let shard_index = *cursor & shard_mask as usize;
        *cursor = cursor.wrapping_add(1);
        if let Ok(mut guard) = shards[shard_index].items.lock() {
            guard.extend(items);
        }
    });
}

fn drain_all<T>(
    config: &Config,
    send_func: &SendFunc<T>,
    shards: &[CentralShard<T>],
    metrics: &Metrics,
    wal: &Option<Arc<Wal<T>>>,
    wal_pending: &AtomicUsize,
    cursor: &mut usize,
    shard_mask: u64,
    batch_size: usize,
) where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    loop {
        let batch = collect_batch(shards, metrics, batch_size, cursor, shard_mask);
        if batch.is_empty() {
            break;
        }
        if send_with_retry(send_func, config, &batch).is_ok() {
            metrics
                .dequeued_total
                .fetch_add(batch.len() as u64, Ordering::Relaxed);
            metrics.last_flush_millis.store(now_millis(), Ordering::Relaxed);
        } else if let Some(wal) = wal.as_ref() {
            let _ = wal.write_batch(&batch);
            wal_pending.fetch_add(batch.len(), Ordering::Relaxed);
        } else {
            metrics
                .dropped_total
                .fetch_add(batch.len() as u64, Ordering::Relaxed);
        }
    }
    if wal_pending.load(Ordering::Relaxed) > 0 {
        let mut avg_latency_ns = 0;
        let mut batch_size_local = batch_size;
        let _ = process_wal(
            config,
            send_func,
            wal,
            wal_pending,
            metrics,
            &mut avg_latency_ns,
            &mut batch_size_local,
        );
    }
}

fn process_wal<T>(
    config: &Config,
    send_func: &SendFunc<T>,
    wal: &Option<Arc<Wal<T>>>,
    wal_pending: &AtomicUsize,
    metrics: &Metrics,
    avg_latency_ns: &mut u64,
    batch_size: &mut usize,
) -> Result<(), QueueError>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    let wal = match wal {
        Some(wal) => wal,
        None => return Ok(()),
    };
    let items = wal.read_and_clear()?;
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
            wal.write_batch(chunk)?;
            wal_pending.fetch_add(chunk.len(), Ordering::Relaxed);
        }
    }
    Ok(())
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

pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as i64
}

pub(crate) fn millis_to_time(value: i64) -> Option<SystemTime> {
    if value <= 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_millis(value as u64))
}
