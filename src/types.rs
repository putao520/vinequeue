use std::sync::Arc;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueErrorKind {
    QueueClosed,
    BufferFull,
    WalWriteFailed,
    SendFailed,
    InvalidConfig,
    SerializationFailed,
    DeserializationFailed,
    ChecksumMismatch,
    IoError,
}

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("queue is closed")]
    QueueClosed,
    #[error("buffer is full")]
    BufferFull,
    #[error("wal write failed")]
    WalWriteFailed,
    #[error("send failed")]
    SendFailed,
    #[error("invalid configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("serialization failed: {0}")]
    SerializationFailed(String),
    #[error("deserialization failed: {0}")]
    DeserializationFailed(String),
    #[error("checksum mismatch")]
    ChecksumMismatch,
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
}

impl QueueError {
    pub fn kind(&self) -> QueueErrorKind {
        match self {
            QueueError::QueueClosed => QueueErrorKind::QueueClosed,
            QueueError::BufferFull => QueueErrorKind::BufferFull,
            QueueError::WalWriteFailed => QueueErrorKind::WalWriteFailed,
            QueueError::SendFailed => QueueErrorKind::SendFailed,
            QueueError::InvalidConfig(_) => QueueErrorKind::InvalidConfig,
            QueueError::SerializationFailed(_) => QueueErrorKind::SerializationFailed,
            QueueError::DeserializationFailed(_) => QueueErrorKind::DeserializationFailed,
            QueueError::ChecksumMismatch => QueueErrorKind::ChecksumMismatch,
            QueueError::IoError(_) => QueueErrorKind::IoError,
        }
    }
}

pub fn is_error(err: &QueueError, target: QueueErrorKind) -> bool {
    err.kind() == target
}

pub fn unwrap_error(err: QueueError) -> QueueError {
    err
}

pub type SendFunc<T> = Arc<dyn Fn(Vec<T>) -> Result<(), QueueError> + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub struct Stats {
    pub enqueued_total: u64,
    pub dequeued_total: u64,
    pub dropped_total: u64,
    pub overflow_count: u64,
    pub utilization_pct: f32,
    pub pending_count: usize,
    pub last_flush_time: Option<SystemTime>,
    pub total_items: usize,
    pub memory_size: usize,
    pub local_buffers: usize,
    pub shard_count: usize,
    pub last_alert_time: Option<SystemTime>,
}

impl Stats {
    pub fn empty() -> Self {
        Self {
            enqueued_total: 0,
            dequeued_total: 0,
            dropped_total: 0,
            overflow_count: 0,
            utilization_pct: 0.0,
            pending_count: 0,
            last_flush_time: None,
            total_items: 0,
            memory_size: 0,
            local_buffers: 0,
            shard_count: 0,
            last_alert_time: None,
        }
    }
}
