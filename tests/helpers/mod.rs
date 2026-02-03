use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fake::faker::internet::en::SafeEmail;
use fake::faker::lorem::en::Word;
use fake::Fake;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TestItem {
    pub id: u64,
    pub payload: String,
    pub tag: String,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub fn fake_item() -> TestItem {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let payload: String = Word().fake();
    let tag: String = SafeEmail().fake();
    TestItem { id, payload, tag }
}

pub fn temp_wal_path(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_nanos();
    path.push(format!("vinequeue-{}-{}.wal", label, nanos));
    path
}

pub fn wait_for(timeout: Duration, mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if check() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for condition");
}
