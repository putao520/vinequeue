use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{QueueError, Stats};

pub trait Queue<T>: Send + Sync
where
    T: Send + Sync + Serialize + DeserializeOwned + Clone + 'static,
{
    fn start(&self) -> Result<(), QueueError>;
    fn stop(&self) -> Result<(), QueueError>;
    fn enqueue(&self, item: T) -> Result<(), QueueError>;
    fn get_stats(&self) -> Stats;
    fn is_started(&self) -> bool;
    fn is_closed(&self) -> bool;
}
