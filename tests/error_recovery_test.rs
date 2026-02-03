use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vinequeue::{Config, HybridQueue, MemoryQueue, Queue, QueueError, Wal};

mod helpers;
use helpers::{fake_item, temp_wal_path, wait_for, TestItem};

const WAL_HEADER_SIZE: u64 = 4 + 2 + 1 + 1 + 8 + 4;

// TEST-RECOVERY-RETRY-001
// Covers: VINEQUEUE-003
// Category: Positive
#[test]
fn retry_succeeds_after_transient_failure() {
    let mut config = Config::default_config();
    config.memory_size = 10;
    config.batch_size = 2;
    config.min_batch_size = 1;
    config.max_batch_size = 2;
    config.flush_timeout = Duration::from_millis(20);
    config.send_timeout = Duration::from_secs(1);
    config.retry_limit = 2;
    config.backpressure_delay = Duration::from_millis(0);

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = Arc::clone(&attempts);

    let queue = MemoryQueue::new(config, move |_batch| {
        let attempt = attempts_clone.fetch_add(1, Ordering::SeqCst) + 1;
        if attempt <= 2 {
            Err(QueueError::SendFailed)
        } else {
            Ok(())
        }
    })
    .expect("create queue");

    queue.start().expect("start");
    queue.enqueue(fake_item()).expect("enqueue");
    queue.enqueue(fake_item()).expect("enqueue");

    wait_for(Duration::from_secs(2), || queue.get_stats().dequeued_total >= 2);
    queue.stop().expect("stop");

    let stats = queue.get_stats();
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(stats.enqueued_total, 2);
    assert_eq!(stats.dequeued_total, 2);
    assert_eq!(stats.dropped_total, 0);
}

// TEST-RECOVERY-BACKPRESSURE-001
// Covers: VINEQUEUE-005
// Category: Boundary + Negative
#[test]
fn backpressure_delays_and_hard_limit_rejects() {
    let mut config = Config::default_config();
    config.memory_size = 4;
    config.soft_limit_pct = 50;
    config.hard_limit_pct = 75;
    config.backpressure_delay = Duration::from_millis(30);
    config.batch_size = 1;
    config.min_batch_size = 1;
    config.max_batch_size = 1;
    config.flush_timeout = Duration::from_millis(50);
    config.send_timeout = Duration::from_secs(1);
    config.retry_limit = 0;

    let queue = MemoryQueue::new(config, |_batch| Ok(())).expect("create queue");

    queue.enqueue(fake_item()).expect("enqueue");
    let start = Instant::now();
    queue.enqueue(fake_item()).expect("enqueue");
    let elapsed = start.elapsed();

    let third = queue.enqueue(fake_item());
    let fourth = queue.enqueue(fake_item());

    queue.stop().expect("stop");
    let stats = queue.get_stats();

    assert!(elapsed >= Duration::from_millis(20));
    assert!(third.is_ok());
    assert!(matches!(fourth, Err(QueueError::BufferFull)));
    assert!(stats.overflow_count >= 1);
    assert!(stats.last_alert_time.is_some());
}

// TEST-RECOVERY-WAL-001
// Covers: VINEQUEUE-002
// Category: Positive
#[test]
fn wal_recovery_replays_persisted_items() {
    let mut config = Config::default_config();
    config.memory_size = 1;
    config.soft_limit_pct = 10;
    config.hard_limit_pct = 50;
    config.backpressure_delay = Duration::from_millis(0);
    config.batch_size = 1;
    config.min_batch_size = 1;
    config.max_batch_size = 1;
    config.flush_timeout = Duration::from_millis(20);
    config.send_timeout = Duration::from_secs(1);
    config.retry_limit = 0;
    config.wal_path = temp_wal_path("recovery");

    let wal_path = config.wal_path.clone();
    let queue = HybridQueue::new(config.clone(), |_batch| Ok(())).expect("create queue");

    let items = vec![fake_item(), fake_item()];
    for item in items.iter().cloned() {
        queue.enqueue(item).expect("enqueue");
    }
    queue.stop().expect("stop");

    let recovered: Arc<Mutex<Vec<TestItem>>> = Arc::new(Mutex::new(Vec::new()));
    let collector = Arc::clone(&recovered);
    let queue = HybridQueue::new(config.clone(), move |batch| {
        collector.lock().expect("lock").extend(batch);
        Ok(())
    })
    .expect("create queue");

    queue.start().expect("start");
    wait_for(Duration::from_secs(2), || recovered.lock().expect("lock").len() == items.len());
    queue.stop().expect("stop");

    let stats = queue.get_stats();
    let recovered_items = recovered.lock().expect("lock").clone();
    let expected_ids: HashSet<u64> = items.iter().map(|item| item.id).collect();
    let recovered_ids: HashSet<u64> = recovered_items.iter().map(|item| item.id).collect();
    let wal = Wal::<TestItem>::open(&config).expect("open wal");

    assert_eq!(recovered_items.len(), items.len());
    assert_eq!(recovered_ids, expected_ids);
    assert_eq!(stats.pending_count, 0);
    assert_eq!(wal.record_count().expect("record count"), 0);

    let _ = std::fs::remove_file(wal_path);
}

// TEST-RECOVERY-NETWORK-001
// Covers: VINEQUEUE-003, VINEQUEUE-002
// Category: Negative + Recovery
#[test]
fn network_failure_routes_to_wal_and_recovers() {
    let mut config = Config::default_config();
    config.memory_size = 10;
    config.batch_size = 2;
    config.min_batch_size = 1;
    config.max_batch_size = 2;
    config.flush_timeout = Duration::from_millis(20);
    config.send_timeout = Duration::from_secs(1);
    config.retry_limit = 0;
    config.backpressure_delay = Duration::from_millis(0);
    config.wal_path = temp_wal_path("network");

    let wal_path = config.wal_path.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = Arc::clone(&attempts);
    let delivered: Arc<Mutex<Vec<TestItem>>> = Arc::new(Mutex::new(Vec::new()));
    let delivered_clone = Arc::clone(&delivered);

    let queue = HybridQueue::new(config.clone(), move |batch| {
        let attempt = attempts_clone.fetch_add(1, Ordering::SeqCst) + 1;
        if attempt == 1 {
            return Err(QueueError::SendFailed);
        }
        delivered_clone.lock().expect("lock").extend(batch);
        Ok(())
    })
    .expect("create queue");

    queue.start().expect("start");
    queue.enqueue(fake_item()).expect("enqueue");
    queue.enqueue(fake_item()).expect("enqueue");

    wait_for(Duration::from_secs(2), || delivered.lock().expect("lock").len() == 2);
    queue.stop().expect("stop");

    let stats = queue.get_stats();
    let wal = Wal::<TestItem>::open(&config).expect("open wal");

    assert!(attempts.load(Ordering::SeqCst) >= 2);
    assert_eq!(stats.dequeued_total, 2);
    assert_eq!(stats.dropped_total, 0);
    assert_eq!(wal.record_count().expect("record count"), 0);

    let _ = std::fs::remove_file(wal_path);
}

// TEST-RECOVERY-WAL-SEC-001
// Covers: VINEQUEUE-002
// Category: Security
#[test]
fn wal_detects_tampered_records() {
    let mut config = Config::default_config();
    config.wal_path = temp_wal_path("tamper");

    let wal = Wal::<TestItem>::open(&config).expect("open wal");
    wal.write(&fake_item()).expect("write wal");

    assert_eq!(wal.record_count().expect("record count"), 1);
    assert!(wal.size().expect("wal size") > WAL_HEADER_SIZE);

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&config.wal_path)
        .expect("open wal file");
    let payload_offset = WAL_HEADER_SIZE + 8 + 8 + 4;
    file.seek(SeekFrom::Start(payload_offset))
        .expect("seek payload");
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte).expect("read payload byte");
    file.seek(SeekFrom::Start(payload_offset))
        .expect("seek payload");
    byte[0] ^= 0xFF;
    file.write_all(&byte).expect("write payload byte");
    file.flush().expect("flush payload");

    let result = wal.read_all();

    assert!(matches!(result, Err(QueueError::ChecksumMismatch)));

    let _ = std::fs::remove_file(config.wal_path);
}
