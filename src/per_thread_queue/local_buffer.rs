use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

thread_local! {
    static LOCAL_INDICES: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());
}

const STATE_IDLE: u8 = 0;
const STATE_WRITING: u8 = 1;
const STATE_DRAINING: u8 = 2;

pub(crate) struct LocalBuffer<T> {
    items: Vec<MaybeUninit<T>>,
    count: usize,
}

impl<T> LocalBuffer<T> {
    fn new(size: usize) -> Self {
        let mut items = Vec::with_capacity(size);
        items.resize_with(size, MaybeUninit::uninit);
        Self {
            items,
            count: 0,
        }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn push(&mut self, item: T) {
        unsafe {
            let ptr = self.items.get_unchecked_mut(self.count).as_mut_ptr();
            ptr.write(item);
        }
        self.count += 1;
    }

    pub(crate) fn drain_into(&mut self, target: &mut Vec<T>) {
        for i in 0..self.count {
            unsafe {
                let value = self.items.get_unchecked(i).assume_init_read();
                target.push(value);
            }
        }
        self.count = 0;
    }
}

impl<T> LocalBuffer<T> {
    pub(crate) fn len(&self) -> usize {
        self.count
    }
}

impl<T> Drop for LocalBuffer<T> {
    fn drop(&mut self) {
        for i in 0..self.count {
            unsafe {
                self.items.get_unchecked_mut(i).assume_init_drop();
            }
        }
    }
}

struct LocalBufferSlot<T> {
    buffer: std::cell::UnsafeCell<LocalBuffer<T>>,
    state: AtomicU8,
}

unsafe impl<T: Send> Sync for LocalBufferSlot<T> {}

pub(crate) struct LocalBuffers<T> {
    queue_id: u64,
    slots: Vec<LocalBufferSlot<T>>,
    next_index: AtomicUsize,
    registry: Mutex<HashMap<std::thread::ThreadId, usize>>,
}

impl<T> LocalBuffers<T> {
    pub(crate) fn new(queue_id: u64, buffer_count: usize, buffer_size: usize) -> Self {
        let mut slots = Vec::with_capacity(buffer_count);
        for _ in 0..buffer_count {
            slots.push(LocalBufferSlot {
                buffer: std::cell::UnsafeCell::new(LocalBuffer::new(buffer_size)),
                state: AtomicU8::new(STATE_IDLE),
            });
        }
        Self {
            queue_id,
            slots,
            next_index: AtomicUsize::new(0),
            registry: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn local_index(&self) -> Option<usize> {
        let queue_id = self.queue_id;
        LOCAL_INDICES.with(|indices| {
            if let Some(idx) = indices.borrow().get(&queue_id) {
                return Some(*idx);
            }
            let mut registry = self.registry.lock().ok()?;
            let thread_id = thread::current().id();
            if let Some(idx) = registry.get(&thread_id) {
                indices.borrow_mut().insert(queue_id, *idx);
                return Some(*idx);
            }
            let idx = self.next_index.fetch_add(1, Ordering::Relaxed);
            if idx >= self.slots.len() {
                return None;
            }
            registry.insert(thread_id, idx);
            indices.borrow_mut().insert(queue_id, idx);
            Some(idx)
        })
    }

    pub(crate) fn with_buffer<R>(
        &self,
        index: usize,
        action: impl FnOnce(&mut LocalBuffer<T>) -> R,
    ) -> Option<R> {
        let slot = self.slots.get(index)?;
        if slot
            .state
            .compare_exchange(STATE_IDLE, STATE_WRITING, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return None;
        }
        let result = unsafe { action(&mut *slot.buffer.get()) };
        slot.state.store(STATE_IDLE, Ordering::Release);
        Some(result)
    }

    pub(crate) fn try_drain(&self, index: usize) -> Option<Vec<T>> {
        let slot = self.slots.get(index)?;
        if slot
            .state
            .compare_exchange(STATE_IDLE, STATE_DRAINING, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return None;
        }
        let mut drained = Vec::new();
        unsafe {
            let buffer = &mut *slot.buffer.get();
            if buffer.count > 0 {
                buffer.drain_into(&mut drained);
            }
        }
        slot.state.store(STATE_IDLE, Ordering::Release);
        if drained.is_empty() {
            None
        } else {
            Some(drained)
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn flush_all(&self, mut push: impl FnMut(Vec<T>)) {
        for idx in 0..self.slots.len() {
            if let Some(drained) = self.try_drain(idx) {
                push(drained);
            }
        }
    }
}
