use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use vinequeue::{Config, MemoryQueue, Queue};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Item(u64);

#[test]
fn flush_timeout_sends_partial_batch() {
    let mut config = Config::default_config();
    config.memory_size = 100;
    config.batch_size = 4;
    config.min_batch_size = 1;
    config.max_batch_size = 4;
    config.flush_timeout = Duration::from_millis(40);
    config.send_timeout = Duration::from_secs(1);
    config.retry_limit = 0;
    config.backpressure_delay = Duration::from_millis(0);

    let batches: Arc<Mutex<Vec<Vec<Item>>>> = Arc::new(Mutex::new(Vec::new()));
    let collector = Arc::clone(&batches);

    let queue = MemoryQueue::new(config, move |batch| {
        collector.lock().unwrap().push(batch);
        Ok(())
    })
    .expect("create queue");

    queue.start().expect("start");
    queue.enqueue(Item(1)).expect("enqueue");

    std::thread::sleep(Duration::from_millis(80));
    queue.stop().expect("stop");

    let batches = batches.lock().unwrap();
    assert!(!batches.is_empty(), "expected at least one batch");
    assert_eq!(batches.iter().map(|b| b.len()).sum::<usize>(), 1);
}
