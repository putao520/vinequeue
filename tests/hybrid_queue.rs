use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use vinequeue::{Config, HybridQueue, Queue};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Item(u64);

fn temp_path(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("vinequeue-{}-{}.wal", label, nanos));
    path
}

#[test]
fn hybrid_queue_flushes_wal_on_stop() {
    let mut config = Config::default_config();
    config.memory_size = 5;
    config.batch_size = 2;
    config.min_batch_size = 1;
    config.max_batch_size = 2;
    config.flush_timeout = Duration::from_millis(20);
    config.send_timeout = Duration::from_secs(1);
    config.retry_limit = 0;
    config.backpressure_delay = Duration::from_millis(0);
    config.wal_path = temp_path("hybrid");

    let wal_path = config.wal_path.clone();
    let batches: Arc<Mutex<Vec<Vec<Item>>>> = Arc::new(Mutex::new(Vec::new()));
    let collector = Arc::clone(&batches);

    let queue = HybridQueue::new(config, move |batch| {
        collector.lock().unwrap().push(batch);
        Ok(())
    })
    .expect("create queue");

    queue.start().expect("start");
    for i in 0..10 {
        queue.enqueue(Item(i)).expect("enqueue");
    }
    queue.stop().expect("stop");

    let total: usize = batches
        .lock()
        .unwrap()
        .iter()
        .map(|batch| batch.len())
        .sum();
    assert_eq!(total, 10);

    let _ = std::fs::remove_file(wal_path);
}
