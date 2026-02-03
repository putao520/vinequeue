use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use vinequeue::{Config, Wal};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Event {
    id: u64,
    name: String,
}

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
fn wal_write_read_clear_cycle() {
    let mut config = Config::default_config();
    config.wal_path = temp_path("wal");
    let wal = Wal::<Event>::open(&config).expect("open wal");

    let items = vec![
        Event {
            id: 1,
            name: "alpha".to_string(),
        },
        Event {
            id: 2,
            name: "beta".to_string(),
        },
    ];

    wal.write_batch(&items).expect("write batch");
    let loaded = wal.read_all().expect("read all");
    assert_eq!(loaded, items);

    wal.clear().expect("clear");
    let empty = wal.read_all().expect("read empty");
    assert!(empty.is_empty());

    let _ = std::fs::remove_file(config.wal_path);
}
