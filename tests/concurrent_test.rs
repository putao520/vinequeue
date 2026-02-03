use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use vinequeue::{Config, MemoryQueue, Queue};

mod helpers;
use helpers::{fake_item, wait_for, TestItem};

// TEST-CONCURRENT-ENQUEUE-001
// Covers: VINEQUEUE-001
// Category: Positive
#[test]
fn concurrent_enqueues_deliver_all_unique_items() {
    let mut config = Config::default_config();
    config.memory_size = 2048;
    config.batch_size = 16;
    config.min_batch_size = 1;
    config.max_batch_size = 16;
    config.flush_timeout = Duration::from_millis(20);
    config.send_timeout = Duration::from_secs(1);
    config.retry_limit = 0;
    config.backpressure_delay = Duration::from_millis(0);

    let collected: Arc<Mutex<Vec<TestItem>>> = Arc::new(Mutex::new(Vec::new()));
    let collector = Arc::clone(&collected);

    let queue = Arc::new(
        MemoryQueue::new(config, move |batch| {
            collector.lock().expect("lock").extend(batch);
            Ok(())
        })
        .expect("create queue"),
    );

    queue.start().expect("start");

    let threads = 4;
    let per_thread = 50;
    let expected = threads * per_thread;
    let (done_tx, done_rx) = mpsc::channel();

    for _ in 0..threads {
        let queue = Arc::clone(&queue);
        let done_tx = done_tx.clone();
        thread::spawn(move || {
            for _ in 0..per_thread {
                queue.enqueue(fake_item()).expect("enqueue");
            }
            done_tx.send(()).expect("send done");
        });
    }
    drop(done_tx);

    for _ in 0..threads {
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("thread completion");
    }

    wait_for(Duration::from_secs(2), || collected.lock().expect("lock").len() == expected);
    queue.stop().expect("stop");

    let stats = queue.get_stats();
    let items = collected.lock().expect("lock").clone();
    let unique_ids: HashSet<u64> = items.iter().map(|item| item.id).collect();

    assert_eq!(items.len(), expected);
    assert_eq!(unique_ids.len(), expected);
    assert_eq!(stats.enqueued_total, expected as u64);
    assert_eq!(stats.dequeued_total, expected as u64);
}

// TEST-CONCURRENT-READWRITE-001
// Covers: VINEQUEUE-001, VINEQUEUE-003
// Category: Positive
#[test]
fn read_write_concurrency_keeps_stats_consistent() {
    let mut config = Config::default_config();
    config.memory_size = 4096;
    config.batch_size = 8;
    config.min_batch_size = 1;
    config.max_batch_size = 8;
    config.flush_timeout = Duration::from_millis(20);
    config.send_timeout = Duration::from_secs(1);
    config.retry_limit = 0;
    config.backpressure_delay = Duration::from_millis(0);

    let queue = Arc::new(
        MemoryQueue::new(config, |_batch| Ok(())).expect("create queue"),
    );

    queue.start().expect("start");

    let writers = 3;
    let per_writer = 40;
    let expected = writers * per_writer;
    let (done_tx, done_rx) = mpsc::channel();

    for _ in 0..writers {
        let queue = Arc::clone(&queue);
        let done_tx = done_tx.clone();
        thread::spawn(move || {
            for _ in 0..per_writer {
                queue.enqueue(fake_item()).expect("enqueue");
            }
            done_tx.send(()).expect("send done");
        });
    }
    drop(done_tx);

    let stop_readers = Arc::new(AtomicBool::new(false));
    let reader_queue = Arc::clone(&queue);
    let reader_stop = Arc::clone(&stop_readers);
    let reader = thread::spawn(move || {
        let mut samples = 0u64;
        while !reader_stop.load(Ordering::Relaxed) {
            let stats = reader_queue.get_stats();
            assert!(stats.enqueued_total >= stats.dequeued_total);
            assert!(stats.utilization_pct >= 0.0 && stats.utilization_pct <= 100.0);
            assert!(stats.pending_count <= stats.total_items);
            samples += 1;
            thread::sleep(Duration::from_millis(5));
        }
        samples
    });

    for _ in 0..writers {
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("writer completion");
    }

    stop_readers.store(true, Ordering::Relaxed);
    let samples = reader.join().expect("reader join");

    wait_for(Duration::from_secs(2), || queue.get_stats().dequeued_total >= expected as u64);
    queue.stop().expect("stop");

    let stats = queue.get_stats();

    assert!(samples > 0);
    assert_eq!(stats.enqueued_total, expected as u64);
    assert_eq!(stats.dequeued_total, expected as u64);
    assert_eq!(stats.pending_count, 0);
}
