use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use vinequeue::{Config, MemoryQueue, Queue};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Item(u64);

#[test]
// Note: 性能测试执行时间较长，在CI中默认跳过
// 可通过 `cargo test perf -- --ignored` 手动运行
#[ignore = "Performance test - takes too long for CI"]
fn enqueue_throughput_smoke() {
    let mut config = Config::default_config();
    config.memory_size = 2_000_000;
    config.batch_size = 1000;
    config.flush_timeout = Duration::from_secs(1);
    config.send_timeout = Duration::from_secs(1);
    config.retry_limit = 0;
    config.backpressure_delay = Duration::from_millis(0);

    let queue = MemoryQueue::new(config, |_batch| Ok(())).expect("create queue");
    queue.start().expect("start");

    let iterations = 1_000_000u64;
    let start = Instant::now();
    for i in 0..iterations {
        queue.enqueue(Item(i)).expect("enqueue");
    }
    let elapsed = start.elapsed();
    let ops = iterations as f64 / elapsed.as_secs_f64();
    println!("enqueue ops/s: {:.2}", ops);

    queue.stop().expect("stop");
    assert!(ops > 0.0);
}
