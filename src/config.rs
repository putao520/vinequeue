use std::path::PathBuf;
use std::time::Duration;

use crate::QueueError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalSyncMode {
    Immediate,
    Periodic,
}

impl WalSyncMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            WalSyncMode::Immediate => "immediate",
            WalSyncMode::Periodic => "periodic",
        }
    }

    pub fn parse(value: &str) -> Result<Self, QueueError> {
        match value {
            "immediate" => Ok(WalSyncMode::Immediate),
            "periodic" => Ok(WalSyncMode::Periodic),
            _ => Err(QueueError::InvalidConfig(
                "wal_sync_mode must be 'immediate' or 'periodic'",
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub memory_size: usize,
    pub soft_limit_pct: u8,
    pub hard_limit_pct: u8,
    pub backpressure_delay: Duration,
    pub wal_path: PathBuf,
    pub wal_max_size: u64,
    pub wal_sync_mode: WalSyncMode,
    pub batch_size: usize,
    pub flush_timeout: Duration,
    pub send_timeout: Duration,
    pub retry_limit: usize,
    pub adaptive_batch: bool,
    pub min_batch_size: usize,
    pub max_batch_size: usize,
    pub batch_adjust_ratio: f64,
    pub enable_metrics: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self::default_config()
    }
}

impl Config {
    pub fn default_config() -> Self {
        Self {
            memory_size: 10000,
            soft_limit_pct: 80,
            hard_limit_pct: 95,
            backpressure_delay: Duration::from_millis(1),
            wal_path: PathBuf::from("./vinequeue.wal"),
            wal_max_size: 100 * 1024 * 1024,
            wal_sync_mode: WalSyncMode::Periodic,
            batch_size: 1000,
            flush_timeout: Duration::from_secs(5),
            send_timeout: Duration::from_secs(10),
            retry_limit: 3,
            adaptive_batch: true,
            min_batch_size: 100,
            max_batch_size: 5000,
            batch_adjust_ratio: 0.1,
            enable_metrics: true,
        }
    }

    pub fn test_config() -> Self {
        Self {
            memory_size: 50000,
            wal_path: PathBuf::from("./test.wal"),
            wal_max_size: 500 * 1024 * 1024,
            ..Self::default_config()
        }
    }

    pub fn high_throughput_config() -> Self {
        Self {
            memory_size: 100000,
            backpressure_delay: Duration::from_millis(2),
            wal_path: PathBuf::from("./data.wal"),
            batch_size: 2000,
            flush_timeout: Duration::from_secs(3),
            min_batch_size: 500,
            max_batch_size: 10000,
            ..Self::default_config()
        }
    }

    pub fn low_latency_config() -> Self {
        Self {
            memory_size: 5000,
            wal_path: PathBuf::from("./cache.wal"),
            batch_size: 100,
            flush_timeout: Duration::from_secs(1),
            adaptive_batch: false,
            min_batch_size: 100,
            max_batch_size: 100,
            ..Self::default_config()
        }
    }

    pub fn validate(&self) -> Result<(), QueueError> {
        if self.memory_size == 0 {
            return Err(QueueError::InvalidConfig("memory_size must be positive"));
        }
        if !(1..=100).contains(&self.soft_limit_pct) {
            return Err(QueueError::InvalidConfig(
                "soft_limit_pct must be between 1 and 100",
            ));
        }
        if !(1..=100).contains(&self.hard_limit_pct) {
            return Err(QueueError::InvalidConfig(
                "hard_limit_pct must be between 1 and 100",
            ));
        }
        if self.hard_limit_pct <= self.soft_limit_pct {
            return Err(QueueError::InvalidConfig(
                "hard_limit_pct must be greater than soft_limit_pct",
            ));
        }
        if self.batch_size == 0 {
            return Err(QueueError::InvalidConfig("batch_size must be positive"));
        }
        if self.min_batch_size == 0 {
            return Err(QueueError::InvalidConfig("min_batch_size must be positive"));
        }
        if self.max_batch_size == 0 {
            return Err(QueueError::InvalidConfig("max_batch_size must be positive"));
        }
        if self.min_batch_size > self.batch_size {
            return Err(QueueError::InvalidConfig(
                "min_batch_size cannot be greater than batch_size",
            ));
        }
        if self.batch_size > self.max_batch_size {
            return Err(QueueError::InvalidConfig(
                "batch_size cannot be greater than max_batch_size",
            ));
        }
        if self.wal_max_size == 0 {
            return Err(QueueError::InvalidConfig("wal_max_size must be positive"));
        }
        if !(0.0..=1.0).contains(&self.batch_adjust_ratio) {
            return Err(QueueError::InvalidConfig(
                "batch_adjust_ratio must be between 0.0 and 1.0",
            ));
        }
        if self.flush_timeout.is_zero() {
            return Err(QueueError::InvalidConfig("flush_timeout must be positive"));
        }
        if self.send_timeout.is_zero() {
            return Err(QueueError::InvalidConfig("send_timeout must be positive"));
        }
        Ok(())
    }
}
