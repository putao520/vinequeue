use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{Config, Queue, QueueError, SendFunc, Stats, Wal};

use super::core::{CoreQueue, OverflowPolicy};

enum QueueType {
    Memory,
    Hybrid,
}

fn build_core<T>(
    config: Config,
    send_func: SendFunc<T>,
    queue_type: QueueType,
) -> Result<CoreQueue<T>, QueueError>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    match queue_type {
        QueueType::Memory => CoreQueue::new(config, send_func, None, OverflowPolicy::Reject),
        QueueType::Hybrid => {
            let wal = Arc::new(Wal::open(&config)?);
            CoreQueue::new(config, send_func, Some(wal), OverflowPolicy::Wal)
        }
    }
}

pub struct MemoryQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    core: Arc<CoreQueue<T>>,
}

impl<T> MemoryQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    pub fn new(
        config: Config,
        send_func: impl Fn(Vec<T>) -> Result<(), QueueError> + Send + Sync + 'static,
    ) -> Result<Self, QueueError> {
        let core = build_core(config, Arc::new(send_func), QueueType::Memory)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }
}

impl<T> Queue<T> for MemoryQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    fn start(&self) -> Result<(), QueueError> {
        self.core.start()
    }

    fn stop(&self) -> Result<(), QueueError> {
        self.core.stop()
    }

    fn enqueue(&self, item: T) -> Result<(), QueueError> {
        self.core.enqueue(item)
    }

    fn get_stats(&self) -> Stats {
        self.core.stats()
    }

    fn is_started(&self) -> bool {
        self.core.is_started()
    }

    fn is_closed(&self) -> bool {
        self.core.is_closed()
    }
}

impl<T> Drop for MemoryQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    fn drop(&mut self) {
        let _ = self.core.stop();
    }
}

pub struct HybridQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    core: Arc<CoreQueue<T>>,
}

impl<T> HybridQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    pub fn new(
        config: Config,
        send_func: impl Fn(Vec<T>) -> Result<(), QueueError> + Send + Sync + 'static,
    ) -> Result<Self, QueueError> {
        let core = build_core(config, Arc::new(send_func), QueueType::Hybrid)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }
}

impl<T> Queue<T> for HybridQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    fn start(&self) -> Result<(), QueueError> {
        self.core.start()
    }

    fn stop(&self) -> Result<(), QueueError> {
        self.core.stop()
    }

    fn enqueue(&self, item: T) -> Result<(), QueueError> {
        self.core.enqueue(item)
    }

    fn get_stats(&self) -> Stats {
        self.core.stats()
    }

    fn is_started(&self) -> bool {
        self.core.is_started()
    }

    fn is_closed(&self) -> bool {
        self.core.is_closed()
    }
}

impl<T> Drop for HybridQueue<T>
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    fn drop(&mut self) {
        let _ = self.core.stop();
    }
}
