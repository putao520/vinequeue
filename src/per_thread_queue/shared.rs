use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize};
use std::sync::Mutex;

pub(crate) struct CentralShard<T> {
    pub(crate) items: Mutex<Vec<T>>,
}

impl<T> CentralShard<T> {
    pub(crate) fn new() -> Self {
        Self {
            items: Mutex::new(Vec::new()),
        }
    }
}

pub(crate) struct Metrics {
    pub(crate) enqueued_total: AtomicU64,
    pub(crate) dequeued_total: AtomicU64,
    pub(crate) dropped_total: AtomicU64,
    pub(crate) overflow_count: AtomicU64,
    pub(crate) total_in_memory: AtomicUsize,
    pub(crate) last_flush_millis: AtomicI64,
    pub(crate) last_alert_millis: AtomicI64,
    pub(crate) started: AtomicBool,
    pub(crate) closed: AtomicBool,
    pub(crate) stop_flag: AtomicBool,
}

impl Metrics {
    pub(crate) fn new() -> Self {
        Self {
            enqueued_total: AtomicU64::new(0),
            dequeued_total: AtomicU64::new(0),
            dropped_total: AtomicU64::new(0),
            overflow_count: AtomicU64::new(0),
            total_in_memory: AtomicUsize::new(0),
            last_flush_millis: AtomicI64::new(0),
            last_alert_millis: AtomicI64::new(0),
            started: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            stop_flag: AtomicBool::new(false),
        }
    }
}
