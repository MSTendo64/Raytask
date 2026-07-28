//! Async Task runtime primitives for the RayTask VM event-loop.

use crate::value::Value;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

pub type TaskHandle = Rc<RefCell<TaskInner>>;

#[derive(Debug)]
pub struct TaskInner {
    pub state: TaskState,
    /// Coroutine ids waiting on this task.
    pub waiters: Vec<usize>,
}

#[derive(Debug, Clone)]
pub enum TaskState {
    Pending,
    Ready(Value),
    Failed(String),
}

impl TaskInner {
    pub fn new_pending() -> TaskHandle {
        Rc::new(RefCell::new(TaskInner {
            state: TaskState::Pending,
            waiters: Vec::new(),
        }))
    }

    pub fn new_ready(value: Value) -> TaskHandle {
        Rc::new(RefCell::new(TaskInner {
            state: TaskState::Ready(value),
            waiters: Vec::new(),
        }))
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.state, TaskState::Ready(_) | TaskState::Failed(_))
    }

    pub fn result(&self) -> Option<Result<Value, String>> {
        match &self.state {
            TaskState::Ready(v) => Some(Ok(v.clone())),
            TaskState::Failed(e) => Some(Err(e.clone())),
            TaskState::Pending => None,
        }
    }
}

pub fn complete_task(task: &TaskHandle, value: Value) -> Vec<usize> {
    let mut inner = task.borrow_mut();
    if matches!(inner.state, TaskState::Pending) {
        inner.state = TaskState::Ready(value);
        std::mem::take(&mut inner.waiters)
    } else {
        Vec::new()
    }
}

pub fn fail_task(task: &TaskHandle, err: String) -> Vec<usize> {
    let mut inner = task.borrow_mut();
    if matches!(inner.state, TaskState::Pending) {
        inner.state = TaskState::Failed(err);
        std::mem::take(&mut inner.waiters)
    } else {
        Vec::new()
    }
}

pub fn add_waiter(task: &TaskHandle, co_id: usize) {
    let mut inner = task.borrow_mut();
    if !inner.waiters.contains(&co_id) {
        inner.waiters.push(co_id);
    }
}

#[derive(Debug, Clone)]
pub struct TimerEntry {
    pub when: Instant,
    pub task: TaskHandle,
}

/// Min-heap by Instant via BinaryHeap + Reverse ordering in VM.
pub struct TimerQueue {
    pub entries: Vec<TimerEntry>,
}

impl TimerQueue {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, when: Instant, task: TaskHandle) {
        self.entries.push(TimerEntry { when, task });
        self.entries.sort_by_key(|e| e.when);
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.entries.first().map(|e| e.when)
    }

    /// Fire all timers that are due; returns completed task handles' waiters lists flattened.
    pub fn fire_due(&mut self, now: Instant) -> Vec<(TaskHandle, Vec<usize>)> {
        let mut fired = Vec::new();
        while let Some(front) = self.entries.first() {
            if front.when > now {
                break;
            }
            let entry = self.entries.remove(0);
            let waiters = complete_task(&entry.task, Value::Null);
            fired.push((entry.task, waiters));
        }
        fired
    }
}

#[derive(Debug, Clone)]
pub struct ReadyQueue {
    inner: VecDeque<usize>,
}

impl ReadyQueue {
    pub fn new() -> Self {
        Self {
            inner: VecDeque::new(),
        }
    }

    pub fn push(&mut self, id: usize) {
        if !self.inner.contains(&id) {
            self.inner.push_back(id);
        }
    }

    pub fn pop(&mut self) -> Option<usize> {
        self.inner.pop_front()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
