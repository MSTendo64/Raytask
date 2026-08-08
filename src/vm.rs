//! Bytecode virtual machine with async/await event-loop.

use crate::async_rt::{
    add_waiter, cancel_task, complete_task, fail_task, token_is_cancelled, ReadyQueue, TaskHandle,
    TaskInner, TimerQueue,
};
use crate::bytecode::{Module, Op};
use crate::error::{RuntimeError, RuntimeResult};
use crate::gc::{GcConfig, GcHeap, GcStats};
use crate::stdlib;
use crate::value::{binary_op, FunctionRef, ObjectInstance, UpvalueCell, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

struct CallFrame {
    chunk: usize,
    ip: usize,
    slots_start: usize,
    upvalues: Vec<UpvalueCell>,
}

struct TryFrame {
    chunk: usize,
    handler_ip: usize,
    stack_len: usize,
    frame_len: usize,
}

struct ParkedCo {
    frames: Vec<CallFrame>,
    stack: Vec<Value>,
    try_stack: Vec<TryFrame>,
    /// Task completed when this coroutine's root frame returns.
    task: Option<TaskHandle>,
    /// If set, resume by pushing this task's result onto the stack.
    awaiting: Option<TaskHandle>,
}

/// Aggregated WhenAll join.
struct JoinGroup {
    outer: TaskHandle,
    tasks: Vec<TaskHandle>,
    any: bool,
}

enum StepCtrl {
    Continue,
    /// Entire program finished (root coroutine done, no more work).
    Halt(Value),
    /// Current coroutine suspended or finished; schedule another.
    Yield,
}

pub struct Vm {
    module: Module,
    stack: Vec<Value>,
    frames: Vec<CallFrame>,
    globals: Vec<Value>,
    /// Tracks which global slots have been explicitly set (to distinguish null from uninitialized).
    global_set: Vec<bool>,
    try_stack: Vec<TryFrame>,
    /// Current coroutine id.
    co_id: usize,
    co_task: Option<TaskHandle>,
    parked: HashMap<usize, ParkedCo>,
    ready: ReadyQueue,
    timers: TimerQueue,
    joins: Vec<JoinGroup>,
    token_tasks: HashMap<usize, Vec<TaskHandle>>,
    next_co_id: usize,
    /// >0 while running nested sync invoke (LINQ etc.) — await not allowed.
    sync_depth: usize,
    root_done: Option<Value>,
    heap: GcHeap,
    /// DAP / debugger session state.
    debug: Option<VmDebug>,
}

struct VmDebug {
    source: PathBuf,
    /// Normalized path → breakpoint lines (+ optional condition).
    breakpoints: HashMap<String, Vec<DebugBreakpoint>>,
    last_line: Option<usize>,
    last_path: Option<String>,
    started: bool,
    /// Frame depth when step began (for over/out).
    step_depth: usize,
    pause_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct DebugBreakpoint {
    pub line: usize,
    pub condition: Option<String>,
    pub log_message: Option<String>,
}

impl Vm {
    pub fn new(module: Module) -> Self {
        Self::with_gc(module, GcConfig::default())
    }

    pub fn with_gc(module: Module, gc: GcConfig) -> Self {
        let stdlib_enabled = module.stdlib_enabled;
         let n = module.globals.len();
        let mut globals = vec![Value::Null; n.max(64)];
        let mut global_set = vec![false; globals.len()];
        for (i, name) in module.globals.iter().enumerate() {
            if stdlib_enabled {
                if let Some(v) = stdlib::builtin_global(name) {
                    globals[i] = v;
                    global_set[i] = true;
                }
            }
        }
        Self {
            module,
            stack: Vec::with_capacity(256),
            frames: Vec::new(),
            globals,
            global_set,
            try_stack: Vec::new(),
            co_id: 0,
            co_task: None,
            parked: HashMap::new(),
            ready: ReadyQueue::new(),
            timers: TimerQueue::new(),
            joins: Vec::new(),
            token_tasks: HashMap::new(),
            next_co_id: 1,
            sync_depth: 0,
            root_done: None,
            heap: GcHeap::new(gc),
            debug: None,
        }
    }

    pub fn gc_stats(&self) -> GcStats {
        self.heap.stats()
    }

    pub fn run(&mut self) -> RuntimeResult<Value> {
        self.heap.install();
        let result = (|| {
            let main = self.module.main_chunk;
            let local_count = self.module.chunks[main].local_count.max(8);
            self.stack.resize(local_count, Value::Null);
            self.frames.push(CallFrame {
                chunk: main,
                ip: 0,
                slots_start: 0,
                upvalues: vec![],
            });
            self.co_id = 0;
            self.co_task = None;
            self.event_loop()
        })();
        GcHeap::uninstall();
        result
    }

    fn collect_roots(&self) -> Vec<Value> {
        let mut roots = Vec::new();
        roots.extend(self.stack.iter().cloned());
        roots.extend(self.globals.iter().cloned());
        for frame in &self.frames {
            for uv in &frame.upvalues {
                roots.push(uv.borrow().clone());
            }
        }
        for p in self.parked.values() {
            roots.extend(p.stack.iter().cloned());
            for frame in &p.frames {
                for uv in &frame.upvalues {
                    roots.push(uv.borrow().clone());
                }
            }
            if let Some(t) = &p.task {
                if let crate::async_rt::TaskState::Ready(v) = &t.borrow().state {
                    roots.push(v.clone());
                }
            }
        }
        roots
    }

    fn gc_collect(&mut self) -> GcStats {
        let roots = self.collect_roots();
        let stats = self.heap.collect(&roots);
        let finals = self.heap.take_pending_finalizers();
        for obj in finals {
            let _ = self.run_finalizer(&obj);
        }
        stats
    }

    fn gc_maybe(&mut self) {
        if !self.heap.config().enabled {
            return;
        }
        let roots = self.collect_roots();
        self.heap.maybe_collect(&roots);
        let finals = self.heap.take_pending_finalizers();
        for obj in finals {
            let _ = self.run_finalizer(&obj);
        }
    }

    fn run_finalizer(&mut self, obj: &Rc<crate::gc::GcObject>) -> RuntimeResult<()> {
        let (class_index, already) = {
            let o = obj.borrow();
            (o.class_index, o.finalized)
        };
        if already {
            return Ok(());
        }
        obj.borrow_mut().finalized = true;
        let Some(ci) = class_index else {
            return Ok(());
        };
        let Some(dtor) = self.module.classes.get(ci).and_then(|c| c.destructor) else {
            return Ok(());
        };
        let f = FunctionRef {
            name: format!("{}=~new", self.module.classes[ci].name),
            chunk_index: dtor,
            arity: 1,
            defaults: vec![],
            is_async: false,
            upvalues: vec![],
        };
        let this = Value::Object(obj.clone());
        let _ = self.invoke_function(&f, &[this])?;
        Ok(())
    }

    fn event_loop(&mut self) -> RuntimeResult<Value> {
        loop {
            self.fire_timers();
            self.poll_joins();

            if self.frames.is_empty() {
                if !self.resume_next()? {
                    if let Some(deadline) = self.timers.next_deadline() {
                        let now = Instant::now();
                        if deadline > now {
                            std::thread::sleep(deadline.saturating_duration_since(now));
                        }
                        continue;
                    }
                    // Idle: no ready coroutines, no timers.
                    return Ok(self.root_done.take().unwrap_or(Value::Null));
                }
            }

            match self.run_slice()? {
                StepCtrl::Continue => {}
                StepCtrl::Halt(v) => return Ok(v),
                StepCtrl::Yield => {
                    // frames already cleared / parked
                }
            }
        }
    }

    /// Run the current coroutine until it yields, halts, or needs reschedule.
    fn run_slice(&mut self) -> RuntimeResult<StepCtrl> {
        loop {
            if self.frames.is_empty() {
                return Ok(StepCtrl::Yield);
            }
            let chunk_idx = self.frame().chunk;
            let ip = self.frame().ip;
            if ip >= self.module.chunks[chunk_idx].code.len() {
                let frame = self.frames.pop().expect("frame");
                self.stack.truncate(frame.slots_start);
                if self.frames.is_empty() {
                    return self.finish_coroutine(Value::Null);
                }
                self.push(Value::Null);
                continue;
            }

            let op = self.module.chunks[chunk_idx].code[ip];
            self.frame_mut().ip += 1;

            match self.step(op) {
                Ok(StepCtrl::Continue) => continue,
                Ok(other) => return Ok(other),
                Err(e) => {
                    if let Some(handler) = self.try_stack.pop() {
                        self.frames.truncate(handler.frame_len);
                        self.stack.truncate(handler.stack_len);
                        // Push raw exception message string for catch blocks to inspect
                        let msg = match &e {
                            RuntimeError::Exception(s) => s.clone(),
                            other => format!("{}", other),
                        };
                        self.push(Value::String(msg.into()));
                        if let Some(frame) = self.frames.last_mut() {
                            frame.chunk = handler.chunk;
                            frame.ip = handler.handler_ip;
                        }
                        continue;
                    }
                    // Fail associated task if any
                    if let Some(task) = self.co_task.take() {
                        let waiters = fail_task(&task, format!("{}", e));
                        for w in waiters {
                            self.ready.push(w);
                        }
                        self.poll_joins();
                        self.frames.clear();
                        self.stack.clear();
                        self.try_stack.clear();
                        return Ok(StepCtrl::Yield);
                    }
                    return Err(e);
                }
            }
        }
    }

    fn finish_coroutine(&mut self, result: Value) -> RuntimeResult<StepCtrl> {
        if let Some(task) = self.co_task.take() {
            let waiters = if self.task_is_cancelled(&task) {
                cancel_task(&task)
            } else {
                complete_task(&task, result)
            };
            for w in waiters {
                self.ready.push(w);
            }
            self.poll_joins();
            self.stack.clear();
            self.try_stack.clear();
            return Ok(StepCtrl::Yield);
        }
        // Root / bare coroutine
        self.root_done = Some(result.clone());
        self.stack.clear();
        self.try_stack.clear();
        if self.ready.is_empty() && self.timers.next_deadline().is_none() && self.parked.is_empty()
        {
            return Ok(StepCtrl::Halt(result));
        }
        // Still may have parked waiters woken by nothing — if ready empty, halt
        if self.ready.is_empty() && self.timers.next_deadline().is_none() {
            return Ok(StepCtrl::Halt(result));
        }
        Ok(StepCtrl::Yield)
    }

    fn fire_timers(&mut self) {
        let now = Instant::now();
        let fired = self.timers.fire_due(now);
        for (task, waiters) in fired {
            if self.task_is_cancelled(&task) {
                let cancelled = cancel_task(&task);
                for w in cancelled {
                    self.ready.push(w);
                }
            }
            for w in waiters {
                self.ready.push(w);
            }
        }
    }

    fn poll_joins(&mut self) {
        let mut still = Vec::new();
        for join in self.joins.drain(..) {
            let all_ready = join.tasks.iter().all(|t| t.borrow().is_ready());
            let first_ready = join.tasks.iter().find_map(|t| t.borrow().result());
            if join.any {
                if let Some(result) = first_ready {
                    let waiters = match result {
                        Ok(v) => complete_task(&join.outer, v),
                        Err(e) => fail_task(&join.outer, e),
                    };
                    for w in waiters {
                        self.ready.push(w);
                    }
                } else {
                    still.push(join);
                }
            } else if all_ready {
                let mut results = Vec::with_capacity(join.tasks.len());
                let mut err: Option<String> = None;
                for t in &join.tasks {
                    match t.borrow().result() {
                        Some(Ok(v)) => results.push(v),
                        Some(Err(e)) => {
                            err = Some(e);
                            break;
                        }
                        None => unreachable!(),
                    }
                }
                let waiters = if let Some(e) = err {
                    fail_task(&join.outer, e)
                } else {
                    complete_task(
                        &join.outer,
                        crate::gc::alloc_array(results),
                    )
                };
                for w in waiters {
                    self.ready.push(w);
                }
            } else {
                still.push(join);
            }
        }
        self.joins = still;
    }

    fn task_is_cancelled(&self, task: &TaskHandle) -> bool {
        task.borrow()
            .cancel_token
            .as_ref()
            .map(token_is_cancelled)
            .unwrap_or(false)
    }

    fn token_object(cancelled: bool) -> Value {
        crate::gc::alloc_object(ObjectInstance {
            class_name: "CancellationToken".into(),
            fields: HashMap::from([("isCancelled".into(), Value::Bool(cancelled))]),
            class_index: None,
            finalized: false,
        })
    }

    fn token_source_object() -> Value {
        crate::gc::alloc_object(ObjectInstance {
            class_name: "CancellationTokenSource".into(),
            fields: HashMap::from([("token".into(), Self::token_object(false))]),
            class_index: None,
            finalized: false,
        })
    }

    fn group_tasks(group: &Value) -> Vec<TaskHandle> {
        let Value::Object(o) = group else {
            return Vec::new();
        };
        let tasks = o
            .borrow()
            .fields
            .get("tasks")
            .cloned()
            .unwrap_or_else(|| crate::gc::alloc_array(Vec::new()));
        match tasks {
            Value::Array(a) => a
                .borrow()
                .iter()
                .filter_map(|v| match v {
                    Value::Task(t) => Some(t.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn group_token(group: &Value) -> Option<Value> {
        let Value::Object(o) = group else {
            return None;
        };
        o.borrow().fields.get("token").cloned()
    }

    fn token_key(token: &Value) -> Option<usize> {
        match token {
            Value::Object(o) => Some(Rc::as_ptr(o) as usize),
            _ => None,
        }
    }

    fn register_task_token(&mut self, task: &TaskHandle, token: Option<&Value>) {
        let Some(key) = token.and_then(Self::token_key) else {
            return;
        };
        self.token_tasks.entry(key).or_default().push(task.clone());
    }

    fn cancel_registered_token_tasks(&mut self, token: &Value) {
        let Some(key) = Self::token_key(token) else {
            return;
        };
        let Some(tasks) = self.token_tasks.get(&key).cloned() else {
            return;
        };
        for task in tasks {
            let waiters = cancel_task(&task);
            for w in waiters {
                self.ready.push(w);
            }
        }
        self.poll_joins();
    }

    fn push_group_task(group: &Value, task: Value) {
        let Value::Object(o) = group else {
            return;
        };
        let guard = o.borrow_mut();
        let Some(Value::Array(tasks)) = guard.fields.get("tasks").cloned() else {
            return;
        };
        tasks.borrow_mut().push(task);
    }

    fn cancel_group_tasks(&mut self, group: &Value) {
        for task in Self::group_tasks(group) {
            let waiters = cancel_task(&task);
            for w in waiters {
                self.ready.push(w);
            }
        }
    }

    fn create_join_task(
        &mut self,
        args: &[Value],
        any: bool,
        cancel_token: Option<Value>,
    ) -> RuntimeResult<Value> {
        let list = args
            .iter()
            .find_map(|v| match v {
                Value::Array(a) => Some(a.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                RuntimeError::Message(if any {
                    "Task.WhenAny requires an array of Tasks".into()
                } else {
                    "Task.WhenAll requires an array of Tasks".into()
                })
            })?;
        let items: Vec<Value> = list.borrow().clone();
        let mut tasks = Vec::new();
        for v in items {
            match v {
                Value::Task(t) => tasks.push(t),
                other => tasks.push(TaskInner::new_ready(other)),
            }
        }
        let outer = TaskInner::new_pending_with_token(cancel_token.clone());
        self.register_task_token(&outer, cancel_token.as_ref());
        if tasks.is_empty() {
            let _ = if any {
                complete_task(&outer, Value::Null)
            } else {
                complete_task(&outer, crate::gc::alloc_array(Vec::new()))
            };
            return Ok(Value::Task(outer));
        }
        self.joins.push(JoinGroup {
            outer: outer.clone(),
            tasks,
            any,
        });
        self.poll_joins();
        Ok(Value::Task(outer))
    }

    fn park_await(&mut self, task: TaskHandle) {
        add_waiter(&task, self.co_id);
        self.parked.insert(
            self.co_id,
            ParkedCo {
                frames: std::mem::take(&mut self.frames),
                stack: std::mem::take(&mut self.stack),
                try_stack: std::mem::take(&mut self.try_stack),
                task: self.co_task.take(),
                awaiting: Some(task),
            },
        );
    }

    fn resume_next(&mut self) -> RuntimeResult<bool> {
        let Some(id) = self.ready.pop() else {
            return Ok(false);
        };
        let Some(mut parked) = self.parked.remove(&id) else {
            // Spurious ready id
            return self.resume_next();
        };

        if let Some(awaiting) = parked.awaiting.clone() {
            let ready_result = awaiting.borrow().result();
            match ready_result {
                Some(Ok(v)) => {
                    parked.stack.push(v);
                    parked.awaiting = None;
                }
                Some(Err(e)) => {
                    if let Some(task) = parked.task.take() {
                        let waiters = fail_task(&task, e);
                        for w in waiters {
                            self.ready.push(w);
                        }
                        self.poll_joins();
                    }
                    return self.resume_next();
                }
                None => {
                    add_waiter(&awaiting, id);
                    self.parked.insert(id, parked);
                    return self.resume_next();
                }
            }
        }

        self.co_id = id;
        self.frames = parked.frames;
        self.stack = parked.stack;
        self.try_stack = parked.try_stack;
        self.co_task = parked.task;
        Ok(true)
    }

    /// Spawn a coroutine for an async (or Task.Run) function call.
    fn spawn_coroutine(
        &mut self,
        chunk: usize,
        args: Vec<Value>,
        task: TaskHandle,
        upvalues: Vec<UpvalueCell>,
    ) -> usize {
        let id = self.next_co_id;
        self.next_co_id += 1;
        let need = self.module.chunks[chunk].local_count.max(args.len());
        let mut stack = args;
        while stack.len() < need {
            stack.push(Value::Null);
        }
        self.parked.insert(
            id,
            ParkedCo {
                frames: vec![CallFrame {
                    chunk,
                    ip: 0,
                    slots_start: 0,
                    upvalues,
                }],
                stack,
                try_stack: Vec::new(),
                task: Some(task),
                awaiting: None,
            },
        );
        self.ready.push(id);
        id
    }

    fn frame(&self) -> &CallFrame {
        self.frames.last().expect("no frame")
    }

    fn frame_mut(&mut self) -> &mut CallFrame {
        self.frames.last_mut().expect("no frame")
    }

    fn read_byte(&mut self) -> u8 {
        let frame = self.frame();
        let b = self.module.chunks[frame.chunk].code[frame.ip];
        self.frame_mut().ip += 1;
        b
    }

    fn read_u16(&mut self) -> u16 {
        let hi = self.read_byte() as u16;
        let lo = self.read_byte() as u16;
        (hi << 8) | lo
    }

    fn push(&mut self, v: Value) {
        self.stack.push(v);
    }

    fn pop(&mut self) -> RuntimeResult<Value> {
        self.stack.pop().ok_or(RuntimeError::StackUnderflow)
    }

    fn peek(&self, distance: usize) -> RuntimeResult<&Value> {
        self.stack
            .get(self.stack.len() - 1 - distance)
            .ok_or(RuntimeError::StackUnderflow)
    }

    fn step(&mut self, op: u8) -> RuntimeResult<StepCtrl> {
        match op {
            x if x == Op::Constant as u8 => {
                let idx = self.read_byte() as usize;
                let chunk = self.frame().chunk;
                let v = self.module.chunks[chunk].constants[idx].clone();
                self.push(v);
            }
            x if x == Op::Constant16 as u8 => {
                let idx = self.read_u16() as usize;
                let chunk = self.frame().chunk;
                let v = self.module.chunks[chunk].constants[idx].clone();
                self.push(v);
            }
            x if x == Op::Null as u8 => self.push(Value::Null),
            x if x == Op::True as u8 => self.push(Value::Bool(true)),
            x if x == Op::False as u8 => self.push(Value::Bool(false)),
            x if x == Op::Pop as u8 => {
                self.pop()?;
            }
            x if x == Op::Dup as u8 => {
                let v = self.peek(0)?.clone();
                self.push(v);
            }
            x if x == Op::GetLocal as u8 => {
                let slot = self.read_byte() as usize;
                let start = self.frame().slots_start;
                let v = self.stack.get(start + slot).cloned().unwrap_or(Value::Null);
                self.push(v);
            }
            x if x == Op::SetLocal as u8 => {
                let slot = self.read_byte() as usize;
                let start = self.frame().slots_start;
                let v = self.peek(0)?.clone();
                let idx = start + slot;
                if idx >= self.stack.len() {
                    self.stack.resize(idx + 1, Value::Null);
                }
                self.stack[idx] = v;
            }
            x if x == Op::IncLocal as u8 => {
                let slot = self.read_byte() as usize;
                let start = self.frame().slots_start;
                let idx = start + slot;
                if let Some(Value::Int(n)) = self.stack.get_mut(idx) {
                    *n += 1;
                }
            }
            x if x == Op::DecLocal as u8 => {
                let slot = self.read_byte() as usize;
                let start = self.frame().slots_start;
                let idx = start + slot;
                if let Some(Value::Int(n)) = self.stack.get_mut(idx) {
                    *n -= 1;
                }
            }
            x if x == Op::GetGlobal as u8 => {
                let idx = self.read_byte() as usize;
                let v = self.globals.get(idx).cloned().unwrap_or(Value::Null);
                if matches!(v, Value::Null) && self.global_set.get(idx).copied().unwrap_or(false) == false {
                    if let Some(name) = self.module.globals.get(idx) {
                        if self.module.stdlib_enabled {
                            if let Some(b) = stdlib::builtin_global(name) {
                                if idx >= self.globals.len() {
                                    self.globals.resize(idx + 1, Value::Null);
                                    self.global_set.resize(idx + 1, false);
                                }
                                self.globals[idx] = b.clone();
                                self.global_set[idx] = true;
                                self.push(b);
                                return Ok(StepCtrl::Continue);
                            }
                        }
                    }
                    if idx < self.global_set.len() {
                        self.global_set[idx] = true;
                    }
                }
                self.push(v);
            }
            x if x == Op::GetGlobal16 as u8 => {
                let idx = self.read_u16() as usize;
                let v = self.globals.get(idx).cloned().unwrap_or(Value::Null);
                if matches!(v, Value::Null) && self.global_set.get(idx).copied().unwrap_or(false) == false {
                    if let Some(name) = self.module.globals.get(idx) {
                        if self.module.stdlib_enabled {
                            if let Some(b) = stdlib::builtin_global(name) {
                                if idx >= self.globals.len() {
                                    self.globals.resize(idx + 1, Value::Null);
                                    self.global_set.resize(idx + 1, false);
                                }
                                self.globals[idx] = b.clone();
                                self.global_set[idx] = true;
                                self.push(b);
                                return Ok(StepCtrl::Continue);
                            }
                        }
                    }
                    if idx < self.global_set.len() {
                        self.global_set[idx] = true;
                    }
                }
                self.push(v);
            }
            x if x == Op::SetGlobal as u8 => {
                let idx = self.read_byte() as usize;
                let v = self.peek(0)?.clone();
                if idx >= self.globals.len() {
                    self.globals.resize(idx + 1, Value::Null);
                    self.global_set.resize(idx + 1, false);
                }
                self.globals[idx] = v;
                if idx < self.global_set.len() {
                    self.global_set[idx] = true;
                }
            }
            x if x == Op::SetGlobal16 as u8 => {
                let idx = self.read_u16() as usize;
                let v = self.peek(0)?.clone();
                if idx >= self.globals.len() {
                    self.globals.resize(idx + 1, Value::Null);
                    self.global_set.resize(idx + 1, false);
                }
                self.globals[idx] = v;
                if idx < self.global_set.len() {
                    self.global_set[idx] = true;
                }
            }
            x if x == Op::DefineGlobal as u8 => {
                let idx = self.read_byte() as usize;
                let v = self.pop()?;
                if idx >= self.globals.len() {
                    self.globals.resize(idx + 1, Value::Null);
                    self.global_set.resize(idx + 1, false);
                }
                self.globals[idx] = v;
                if idx < self.global_set.len() {
                    self.global_set[idx] = true;
                }
            }
            x if x == Op::DefineGlobal16 as u8 => {
                let idx = self.read_u16() as usize;
                let v = self.pop()?;
                if idx >= self.globals.len() {
                    self.globals.resize(idx + 1, Value::Null);
                    self.global_set.resize(idx + 1, false);
                }
                self.globals[idx] = v;
                if idx < self.global_set.len() {
                    self.global_set[idx] = true;
                }
            }
            x if x == Op::GetProperty as u8 => {
                let key = self.pop()?;
                let obj = self.pop()?;
                let name = key.as_string();
                let getter_fn = if let Value::Object(o) = &obj {
                    let getter = format!("get_{}", name);
                    o.borrow().fields.get(&getter).cloned()
                } else {
                    None
                };
                if let Some(Value::Function(f)) = getter_fn {
                    let result = self.invoke_function(&f, &[obj])?;
                    self.push(result);
                    return Ok(StepCtrl::Continue);
                }
                match get_property(&obj, &name) {
                    Ok(result) => self.push(result),
                    Err(_) => {
                        // Extension / Type.method globals: string.Foo, Class.Foo
                        let type_name = match &obj {
                            Value::Object(o) => o.borrow().class_name.clone(),
                            Value::TypeModule(module) => module.to_string(),
                            Value::String(_) => "string".into(),
                            Value::Array(_) => "List".into(),
                            Value::Int(_) => "int".into(),
                            Value::Float(_) => "double".into(),
                            Value::Bool(_) => "bool".into(),
                            other => other.type_name().to_string(),
                        };
                        let key = format!("{}.{}", type_name, name);
                        if let Value::TypeModule(module) = &obj {
                            if let Some(class) = self.module.classes.iter().find(|c| c.name == **module) {
                                let getter_name = format!("get_{}", name);
                                if let Some((_, chunk_index)) = class
                                    .methods
                                    .iter()
                                    .find(|(method, _)| method.as_str() == getter_name)
                                {
                                    let chunk = &self.module.chunks[*chunk_index];
                                    let result = self.invoke_function(
                                        &FunctionRef {
                                            name: format!("{}.{}", type_name, getter_name),
                                            chunk_index: *chunk_index,
                                            arity: chunk.arity,
                                            defaults: vec![],
                                            is_async: chunk.is_async,
                                            upvalues: vec![],
                                        },
                                        &[],
                                    )?;
                                    self.push(result);
                                    return Ok(StepCtrl::Continue);
                                }
                            }
                        }
                        if let Some(class) = self.module.classes.iter().find(|c| c.name == type_name) {
                            if let Some((_, chunk_index)) = class
                                .methods
                                .iter()
                                .find(|(method, _)| method.as_str() == name)
                            {
                                let chunk = &self.module.chunks[*chunk_index];
                                self.push(Value::Function(FunctionRef {
                                    name: key,
                                    chunk_index: *chunk_index,
                                    arity: chunk.arity,
                                    defaults: vec![],
                                    is_async: chunk.is_async,
                                    upvalues: vec![],
                                }));
                                return Ok(StepCtrl::Continue);
                            }
                        }
                        if let Some(idx) = self.module.globals.iter().position(|g| g == &key) {
                            let v = self.globals.get(idx).cloned().unwrap_or(Value::Null);
                            self.push(v);
                        } else {
                            return Err(RuntimeError::Message(format!(
                                "undefined property '{}.{}'",
                                type_name, name
                            )));
                        }
                    }
                }
            }
            x if x == Op::SetProperty as u8 => {
                let key = self.pop()?;
                let value = self.pop()?;
                let obj = self.pop()?;
                let name = key.as_string();
                let setter_fn = if let Value::Object(o) = &obj {
                    let setter = format!("set_{}", name);
                    o.borrow().fields.get(&setter).cloned()
                } else {
                    None
                };
                if let Some(Value::Function(f)) = setter_fn {
                    let _ = self.invoke_function(&f, &[obj, value.clone()])?;
                    self.push(value);
                    return Ok(StepCtrl::Continue);
                }
                if let Value::TypeModule(module) = &obj {
                    if let Some(class) = self.module.classes.iter().find(|c| c.name == **module) {
                        let setter_name = format!("set_{}", name);
                        if let Some((_, chunk_index)) = class
                            .methods
                            .iter()
                            .find(|(method, _)| method.as_str() == setter_name)
                        {
                            let chunk = &self.module.chunks[*chunk_index];
                            let _ = self.invoke_function(
                                &FunctionRef {
                                    name: format!("{}.{}", module, setter_name),
                                    chunk_index: *chunk_index,
                                    arity: chunk.arity,
                                    defaults: vec![],
                                    is_async: chunk.is_async,
                                    upvalues: vec![],
                                },
                                &[value.clone()],
                            )?;
                            self.push(value);
                            return Ok(StepCtrl::Continue);
                        }
                    }
                    let global_key = format!("{}.{}", module, name);
                    if let Some(idx) = self.module.globals.iter().position(|g| g == &global_key) {
                        if idx >= self.globals.len() {
                            self.globals.resize(idx + 1, Value::Null);
                        }
                        self.globals[idx] = value.clone();
                        self.push(value);
                        return Ok(StepCtrl::Continue);
                    }
                }
                let mut obj = obj;
                set_property(&mut obj, &name, value.clone())?;
                self.push(value);
            }
            x if x == Op::GetIndex as u8 => {
                let index = self.pop()?;
                let obj = self.pop()?;
                // Class indexer: get_Item
                let indexer = if let Value::Object(o) = &obj {
                    o.borrow().fields.get("get_Item").cloned()
                } else {
                    None
                };
                if let Some(Value::Function(f)) = indexer {
                    let result = self.invoke_function(&f, &[obj, index])?;
                    self.push(result);
                    return Ok(StepCtrl::Continue);
                }
                let result = get_index(&obj, &index)?;
                self.push(result);
            }
            x if x == Op::SetIndex as u8 => {
                let value = self.pop()?;
                let index = self.pop()?;
                let obj = self.pop()?;
                let indexer = if let Value::Object(o) = &obj {
                    o.borrow().fields.get("set_Item").cloned()
                } else {
                    None
                };
                if let Some(Value::Function(f)) = indexer {
                    let _ = self.invoke_function(&f, &[obj, index, value.clone()])?;
                    self.push(value);
                    return Ok(StepCtrl::Continue);
                }
                let mut obj = obj;
                set_index(&mut obj, &index, value.clone())?;
                self.push(value);
            }
            x if x == Op::Add as u8 => self.binop("+")?,
            x if x == Op::Sub as u8 => self.binop("-")?,
            x if x == Op::Mul as u8 => self.binop("*")?,
            x if x == Op::Div as u8 => self.binop("/")?,
            x if x == Op::Mod as u8 => self.binop("%")?,
            x if x == Op::Eq as u8 => self.binop("==")?,
            x if x == Op::Ne as u8 => self.binop("!=")?,
            x if x == Op::Lt as u8 => self.binop("<")?,
            x if x == Op::Le as u8 => self.binop("<=")?,
            x if x == Op::Gt as u8 => self.binop(">")?,
            x if x == Op::Ge as u8 => self.binop(">=")?,
            x if x == Op::BitAnd as u8 => self.binop("&")?,
            x if x == Op::BitOr as u8 => self.binop("|")?,
            x if x == Op::BitXor as u8 => self.binop("^")?,
            x if x == Op::Shl as u8 => self.binop("<<")?,
            x if x == Op::Shr as u8 => self.binop(">>")?,
            x if x == Op::NullCoalesce as u8 => self.binop("??")?,
            x if x == Op::And as u8 => self.binop("&&")?,
            x if x == Op::Or as u8 => self.binop("||")?,
            x if x == Op::Neg as u8 => {
                let v = self.pop()?;
                match v {
                    Value::Int(n) => self.push(Value::Int(-n)),
                    Value::Float(n) => self.push(Value::Float(-n)),
                    _ => self.push(Value::Int(-v.as_int()?)),
                }
            }
            x if x == Op::Not as u8 => {
                let v = self.pop()?;
                self.push(Value::Bool(!v.is_truthy()));
            }
            x if x == Op::BitNot as u8 => {
                let v = self.pop()?;
                self.push(Value::Int(!v.as_int()?));
            }
            x if x == Op::IsNull as u8 => {
                let v = self.pop()?;
                self.push(Value::Bool(matches!(v, Value::Null)));
            }
            x if x == Op::ToString as u8 => {
                let v = self.pop()?;
                self.push(Value::String(v.as_string().into()));
            }
            x if x == Op::Jump as u8 => {
                let offset = self.read_u16() as usize;
                self.frame_mut().ip += offset;
            }
            x if x == Op::JumpIfFalse as u8 => {
                let offset = self.read_u16() as usize;
                if !self.peek(0)?.is_truthy() {
                    self.frame_mut().ip += offset;
                }
            }
            x if x == Op::JumpIfTrue as u8 => {
                let offset = self.read_u16() as usize;
                if self.peek(0)?.is_truthy() {
                    self.frame_mut().ip += offset;
                }
            }
            x if x == Op::Loop as u8 => {
                let offset = self.read_u16() as usize;
                self.frame_mut().ip -= offset;
            }
            x if x == Op::Call as u8 => {
                let arg_count = self.read_byte() as usize;
                self.call_value(arg_count)?;
            }
            x if x == Op::Return as u8 => {
                let result = self.pop()?;
                let frame = self.frames.pop().expect("frame");
                self.stack.truncate(frame.slots_start);
                if self.frames.is_empty() {
                    return self.finish_coroutine(result);
                }
                self.push(result);
            }
            x if x == Op::Await as u8 => {
                let v = self.pop()?;
                match v {
                    Value::Task(t) => {
                        let ready_result = t.borrow().result();
                        match ready_result {
                            Some(Ok(r)) => self.push(r),
                            Some(Err(e)) => {
                                return Err(RuntimeError::Exception(e));
                            }
                            None => {
                                if self.sync_depth > 0 {
                                    return Err(RuntimeError::Message(
                                        "cannot await inside a synchronous callback".into(),
                                    ));
                                }
                                self.park_await(t);
                                return Ok(StepCtrl::Yield);
                            }
                        }
                    }
                    other => {
                        // await non-Task is a no-op (identity)
                        self.push(other);
                    }
                }
            }
            x if x == Op::NewObject as u8 => {
                let ci = self.read_byte();
                if ci == 0xff {
                    let name = self.pop()?.as_string();
                    let obj = crate::gc::alloc_object(ObjectInstance {
                        class_name: name,
                        fields: HashMap::new(),
        class_index: None,
        finalized: false,
    });
                    self.push(obj);
                } else {
                    let obj = self.instantiate_class(ci as usize)?;
                    self.push(obj);
                }
            }
            x if x == Op::NewArray as u8 => {
                let count = self.read_byte() as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.pop()?);
                }
                items.reverse();
                self.push(crate::gc::alloc_array(items));
                self.gc_maybe();
            }
            x if x == Op::Print as u8 => {
                let v = self.pop()?;
                crate::debug_io::write_stdout(&v.as_string());
                self.push(Value::Null);
            }
            x if x == Op::Halt as u8 => return Ok(StepCtrl::Halt(Value::Null)),
            x if x == Op::Throw as u8 => {
                let v = self.pop()?;
                return Err(RuntimeError::Exception(v.as_string()));
            }
            x if x == Op::TryBegin as u8 => {
                let offset = self.read_u16() as usize;
                let handler_ip = self.frame().ip + offset;
                self.try_stack.push(TryFrame {
                    chunk: self.frame().chunk,
                    handler_ip,
                    stack_len: self.stack.len(),
                    frame_len: self.frames.len(),
                });
            }
            x if x == Op::TryEnd as u8 => {
                self.try_stack.pop();
            }
            x if x == Op::MakeClosure as u8 => {
                let n = self.read_byte() as usize;
                let mut captures = Vec::with_capacity(n);
                for _ in 0..n {
                    let is_local = self.read_byte() != 0;
                    let idx = self.read_byte() as usize;
                    if is_local {
                        let start = self.frame().slots_start;
                        let v = self
                            .stack
                            .get(start + idx)
                            .cloned()
                            .unwrap_or(Value::Null);
                        captures.push(Rc::new(RefCell::new(v)));
                    } else {
                        let uv = self
                            .frame()
                            .upvalues
                            .get(idx)
                            .cloned()
                            .ok_or_else(|| {
                                RuntimeError::Message(format!("invalid upvalue index {}", idx))
                            })?;
                        captures.push(uv);
                    }
                }
                let proto = self.pop()?;
                match proto {
                    Value::Function(mut f) => {
                        f.upvalues = captures;
                        self.push(Value::Function(f));
                    }
                    other => {
                        return Err(RuntimeError::TypeError(format!(
                            "MakeClosure expects function, got {}",
                            other.type_name()
                        )));
                    }
                }
            }
            x if x == Op::GetUpvalue as u8 => {
                let idx = self.read_byte() as usize;
                let cell = self
                    .frame()
                    .upvalues
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| {
                        RuntimeError::Message(format!("invalid upvalue get {}", idx))
                    })?;
                self.push(cell.borrow().clone());
            }
            x if x == Op::SetUpvalue as u8 => {
                let idx = self.read_byte() as usize;
                let v = self.peek(0)?.clone();
                let cell = self
                    .frame()
                    .upvalues
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| {
                        RuntimeError::Message(format!("invalid upvalue set {}", idx))
                    })?;
                *cell.borrow_mut() = v;
            }
            x if x == Op::IsInstance as u8 => {
                let ty_val = self.pop()?;
                let value = self.pop()?;
                let ok = match ty_val {
                    Value::Type(t) => self.value_is_instance(&value, &t),
                    Value::String(s) => self.value_is_instance(
                        &value,
                        &crate::value::TypeHandle {
                            name: s.to_string(),
                            kind: "type".into(),
                            class_index: self
                                .module
                                .classes
                                .iter()
                                .position(|c| c.name == s.as_ref()),
                            fields: Vec::new(),
                            field_types: Vec::new(),
                            methods: Vec::new(),
                        },
                    ),
                    _ => false,
                };
                self.push(Value::Bool(ok));
            }
            x if x == Op::StringStartsWith as u8 => {
                let prefix = self.pop()?;
                let s = self.pop()?;
                let s_str: &str = &s.as_string();
                let prefix_str: &str = &prefix.as_string();
                let ok = s_str.starts_with(prefix_str) || s_str == prefix_str;
                self.push(Value::Bool(ok));
            }
            other => {
                return Err(RuntimeError::Message(format!("unknown opcode {}", other)));
            }
        }
        Ok(StepCtrl::Continue)
    }

    fn value_is_instance(&self, value: &Value, ty: &crate::value::TypeHandle) -> bool {
        let name = ty.name.as_str();
        match name {
            "null" => return matches!(value, Value::Null),
            "bool" => return matches!(value, Value::Bool(_)),
            "int" | "long" | "short" | "sbyte" | "byte" => {
                return matches!(value, Value::Int(_));
            }
            "uint" | "ulong" | "ushort" => return matches!(value, Value::UInt(_)),
            "float" | "double" | "decimal" => return matches!(value, Value::Float(_)),
            "char" => return matches!(value, Value::Char(_)),
            "string" | "String" => return matches!(value, Value::String(_)),
            "array" | "List" => return matches!(value, Value::Array(_)),
            "dictionary" | "Dictionary" => return matches!(value, Value::Dict(_)),
            "function" => {
                return matches!(value, Value::Function(_) | Value::Native(_) | Value::Ffi(_));
            }
            "Type" => return matches!(value, Value::Type(_)),
            "ptr" => return matches!(value, Value::Ptr(_)),
            "object" | "dyn" => return !matches!(value, Value::Null),
            _ => {}
        }
        match value {
            Value::Object(o) => {
                let o = o.borrow();
                if let Some(want) = ty.class_index {
                    let mut cur = o.class_index;
                    while let Some(i) = cur {
                        if i == want {
                            return true;
                        }
                        cur = self.module.classes.get(i).and_then(|c| c.base);
                    }
                }
                o.class_name == ty.name
            }
            Value::TypeModule(n) => n.as_ref() == ty.name,
            Value::Type(t) => t.name == ty.name,
            _ => value.type_name() == name,
        }
    }

    fn binop(&mut self, op: &str) -> RuntimeResult<()> {
        let right = self.pop()?;
        let left = self.pop()?;
        let overload = if let Value::Object(o) = &left {
            let class_name = o.borrow().class_name.clone();
            let key = format!("{}.operator{}", class_name, op);
            if let Some(idx) = self.module.globals.iter().position(|g| g == &key) {
                if let Value::Function(f) = self.globals[idx].clone() {
                    Some(f)
                } else {
                    None
                }
            } else {
                let op_name = format!("operator{}", op);
                match o.borrow().fields.get(&op_name).cloned() {
                    Some(Value::Function(f)) => Some(f),
                    _ => None,
                }
            }
        } else {
            None
        };
        if let Some(f) = overload {
            self.push(Value::Function(f));
            self.push(left);
            self.push(right);
            self.call_value(2)?;
            return Ok(());
        }
        self.push(binary_op(op, &left, &right)?);
        Ok(())
    }

    fn instantiate_class(&self, ci: usize) -> RuntimeResult<Value> {
        let mut chain = Vec::new();
        let mut cur = Some(ci);
        while let Some(i) = cur {
            chain.push(i);
            cur = self.module.classes.get(i).and_then(|c| c.base);
        }
        chain.reverse();
        let mut fields = HashMap::new();
        for &i in &chain {
            let class = &self.module.classes[i];
            for f in &class.fields {
                fields.entry(f.clone()).or_insert(Value::Null);
            }
            for (name, chunk_idx) in &class.methods {
                fields.insert(
                    name.clone(),
                    Value::Function(FunctionRef {
                        name: name.clone(),
                        chunk_index: *chunk_idx,
                        arity: self.module.chunks[*chunk_idx].arity,
                        defaults: vec![],
                        is_async: self.module.chunks[*chunk_idx].is_async,
                        upvalues: vec![],
                    }),
                );
            }
        }
        let class_name = self.module.classes[ci].name.clone();
        Ok(crate::gc::alloc_object(ObjectInstance {
            class_name,
            fields,
            class_index: Some(ci),
            finalized: false,
        }))
    }

    fn call_value(&mut self, arg_count: usize) -> RuntimeResult<()> {
        let len = self.stack.len();
        if arg_count >= 1 && len >= arg_count + 1 {
            let recv_idx = len - arg_count - 1;
            let fn_idx = len - arg_count;
                    let at_fn_is_callee = matches!(
                        &self.stack[fn_idx],
                        Value::Function(_) | Value::Native(_) | Value::Ffi(_)
                    );
                    let below_is_callee = matches!(
                        &self.stack[recv_idx],
                        Value::Function(_) | Value::Native(_) | Value::Ffi(_)
                    );
            if !below_is_callee && at_fn_is_callee {
                let method = self.stack.remove(fn_idx);
                self.stack.insert(recv_idx, method);
            }
        }

        let callee_idx = self.stack.len() - arg_count - 1;
        let callee = self
            .stack
            .get(callee_idx)
            .cloned()
            .ok_or(RuntimeError::StackUnderflow)?;

        match callee {
            Value::Function(f) => {
                let is_async = f.is_async || self.module.chunks[f.chunk_index].is_async;
                if is_async {
                    let chunk = f.chunk_index;
                    let upvalues = f.upvalues.clone();
                    self.stack.remove(callee_idx);
                    let mut args = Vec::with_capacity(arg_count);
                    for _ in 0..arg_count {
                        args.push(self.pop()?);
                    }
                    args.reverse();
                    let task = TaskInner::new_pending();
                    self.spawn_coroutine(chunk, args, task.clone(), upvalues);
                    self.push(Value::Task(task));
                } else {
                    let chunk = f.chunk_index;
                    let upvalues = f.upvalues.clone();
                    self.stack.remove(callee_idx);
                    let slots_start = callee_idx;
                    let need = self.module.chunks[chunk].local_count.max(arg_count);
                    while self.stack.len() < slots_start + need {
                        self.stack.push(Value::Null);
                    }
                    self.frames.push(CallFrame {
                        chunk,
                        ip: 0,
                        slots_start,
                        upvalues,
                    });
                }
            }
            Value::Native(id) => {
                let mut args = Vec::new();
                for _ in 0..arg_count {
                    args.push(self.pop()?);
                }
                args.reverse();
                self.pop()?; // callee

                if matches!(
                    id,
                    crate::stdlib::ids::LIST_WHERE
                        | crate::stdlib::ids::LIST_SELECT
                        | crate::stdlib::ids::LIST_ANY
                        | crate::stdlib::ids::LIST_ALL
                        | crate::stdlib::ids::LIST_PARALLEL_MAP
                        | crate::stdlib::ids::TASK_RUN
                        | crate::stdlib::ids::TASK_DELAY
                        | crate::stdlib::ids::TASK_WHEN_ALL
                        | crate::stdlib::ids::TASK_WHEN_ANY
                        | crate::stdlib::ids::TASKGROUP_NEW
                        | crate::stdlib::ids::TASKGROUP_RUN
                        | crate::stdlib::ids::TASKGROUP_CANCEL
                        | crate::stdlib::ids::TASKGROUP_WHEN_ALL
                        | crate::stdlib::ids::TASKGROUP_WHEN_ANY
                        | crate::stdlib::ids::CTS_NEW
                        | crate::stdlib::ids::CTS_CANCEL
                        | crate::stdlib::ids::TOKEN_THROW_IF_CANCELLED
                        | crate::stdlib::ids::THREAD_RUN
                        | crate::stdlib::ids::GC_COLLECT
                        | crate::stdlib::ids::GC_STATS
                        | crate::stdlib::ids::TYPE_INVOKE
                ) {
                    let result = self.call_native_with_vm(id, &args)?;
                    self.push(result);
                } else {
                    let result = stdlib::call_native(id, &args)?;
                    self.push(result);
                }
            }
            Value::Ffi(f) => {
                let mut args = Vec::new();
                for _ in 0..arg_count {
                    args.push(self.pop()?);
                }
                args.reverse();
                self.pop()?; // callee
                let result = crate::ffi::call(&f, &args)?;
                self.push(result);
            }
            other => {
                return Err(RuntimeError::TypeError(format!(
                    "cannot call {}",
                    other.type_name()
                )));
            }
        }
        Ok(())
    }

    /// Invoke a Function value with args, running until it returns (nested frames).
    fn invoke_function(&mut self, f: &FunctionRef, args: &[Value]) -> RuntimeResult<Value> {
        if f.is_async || self.module.chunks[f.chunk_index].is_async {
            return Err(RuntimeError::Message(
                "cannot invoke async function synchronously".into(),
            ));
        }
        let frame_depth = self.frames.len();
        self.sync_depth += 1;
        self.push(Value::Function(f.clone()));
        for a in args {
            self.push(a.clone());
        }
        let call_result = self.call_value(args.len());
        if let Err(e) = call_result {
            self.sync_depth -= 1;
            return Err(e);
        }
        let result = (|| -> RuntimeResult<Value> {
            while self.frames.len() > frame_depth {
                let chunk_idx = self.frame().chunk;
                let ip = self.frame().ip;
                if ip >= self.module.chunks[chunk_idx].code.len() {
                    let frame = self.frames.pop().expect("frame");
                    self.stack.truncate(frame.slots_start);
                    self.push(Value::Null);
                    continue;
                }
                let op = self.module.chunks[chunk_idx].code[ip];
                self.frame_mut().ip += 1;
                match self.step(op)? {
                    StepCtrl::Continue => {}
                    StepCtrl::Halt(v) => return Ok(v),
                    StepCtrl::Yield => {
                        return Err(RuntimeError::Message(
                            "cannot await inside a synchronous callback".into(),
                        ));
                    }
                }
            }
            self.pop()
        })();
        self.sync_depth -= 1;
        result
    }

    fn call_native_with_vm(&mut self, id: usize, args: &[Value]) -> RuntimeResult<Value> {
        use crate::stdlib::ids::*;
        match id {
            LIST_WHERE | LIST_SELECT | LIST_ANY | LIST_ALL | LIST_PARALLEL_MAP => {
                let list = args.first().cloned().ok_or(RuntimeError::StackUnderflow)?;
                let pred = args.get(1).cloned();
                let arr = match &list {
                    Value::Array(a) => a.clone(),
                    _ => {
                        return Err(RuntimeError::TypeError("expected List".into()));
                    }
                };
                let items: Vec<Value> = arr.borrow().clone();
                match id {
                    LIST_WHERE => {
                        let pred = pred.ok_or_else(|| {
                            RuntimeError::Message("List.Where requires a predicate".into())
                        })?;
                        let f = match pred {
                            Value::Function(f) => f,
                            _ => {
                                return Err(RuntimeError::TypeError(
                                    "Where predicate must be a function".into(),
                                ));
                            }
                        };
                        let mut out = Vec::new();
                        for item in items {
                            let r = self.invoke_function(&f, &[item.clone()])?;
                            if r.is_truthy() {
                                out.push(item);
                            }
                        }
                        Ok(crate::gc::alloc_array(out))
                    }
                    LIST_SELECT | LIST_PARALLEL_MAP => {
                        let pred = pred.ok_or_else(|| {
                            RuntimeError::Message("List mapper requires a function".into())
                        })?;
                        let f = match pred {
                            Value::Function(f) => f,
                            _ => {
                                return Err(RuntimeError::TypeError(
                                    "mapper must be a function".into(),
                                ));
                            }
                        };
                        // ParallelMap: same semantics as Select in the single-threaded VM
                        // (RayTask Values are not Send); still available for API parity.
                        let mut out = Vec::new();
                        for item in items {
                            out.push(self.invoke_function(&f, &[item])?);
                        }
                        Ok(crate::gc::alloc_array(out))
                    }
                    LIST_ANY => {
                        if let Some(Value::Function(f)) = pred {
                            for item in items {
                                if self.invoke_function(&f, &[item])?.is_truthy() {
                                    return Ok(Value::Bool(true));
                                }
                            }
                            Ok(Value::Bool(false))
                        } else {
                            Ok(Value::Bool(!arr.borrow().is_empty()))
                        }
                    }
                    LIST_ALL => {
                        if let Some(Value::Function(f)) = pred {
                            for item in items {
                                if !self.invoke_function(&f, &[item])?.is_truthy() {
                                    return Ok(Value::Bool(false));
                                }
                            }
                            Ok(Value::Bool(true))
                        } else {
                            Ok(Value::Bool(true))
                        }
                    }
                    _ => unreachable!(),
                }
            }
            TASK_DELAY => {
                // Task.Delay(ms) or Task.Delay(Task, ms)
                let ms = args
                    .iter()
                    .rev()
                    .find_map(|v| v.as_int().ok())
                    .unwrap_or(0)
                    .max(0) as u64;
                let token = args.iter().find_map(|v| match v {
                    Value::Object(o) if o.borrow().class_name == "CancellationToken" => Some(v.clone()),
                    _ => None,
                });
                let task = TaskInner::new_pending_with_token(token.clone());
                self.register_task_token(&task, token.as_ref());
                if token.as_ref().map(token_is_cancelled).unwrap_or(false) {
                    let _ = cancel_task(&task);
                    return Ok(Value::Task(task));
                }
                let when = Instant::now() + Duration::from_millis(ms);
                self.timers.push(when, task.clone());
                Ok(Value::Task(task))
            }
            TASK_RUN => {
                let f = args
                    .iter()
                    .find_map(|v| match v {
                        Value::Function(f) => Some(f.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| RuntimeError::Message("Task.Run requires a function".into()))?;
                let token = args.iter().find_map(|v| match v {
                    Value::Object(o) if o.borrow().class_name == "CancellationToken" => Some(v.clone()),
                    _ => None,
                });
                let task = TaskInner::new_pending_with_token(token.clone());
                self.register_task_token(&task, token.as_ref());
                if token.as_ref().map(token_is_cancelled).unwrap_or(false) {
                    let _ = cancel_task(&task);
                    return Ok(Value::Task(task));
                }
                self.spawn_coroutine(f.chunk_index, vec![], task.clone(), f.upvalues.clone());
                Ok(Value::Task(task))
            }
            THREAD_RUN => {
                // Thread.Run(fn) — run synchronously on a "background" call (no OS thread:
                // Values aren't Send). Returns the function result.
                let f = args
                    .iter()
                    .find_map(|v| match v {
                        Value::Function(f) => Some(f.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| RuntimeError::Message("Thread.Run requires a function".into()))?;
                self.invoke_function(&f, &[])
            }
            TASK_WHEN_ALL => {
                self.create_join_task(args, false, None)
            }
            TASK_WHEN_ANY => {
                self.create_join_task(args, true, None)
            }
            CTS_NEW => Ok(Self::token_source_object()),
            CTS_CANCEL => {
                if let Some(Value::Object(source)) = args.iter().find(|v| matches!(v, Value::Object(_))) {
                    if let Some(Value::Object(token)) = source.borrow().fields.get("token").cloned() {
                        token
                            .borrow_mut()
                            .fields
                            .insert("isCancelled".into(), Value::Bool(true));
                        self.cancel_registered_token_tasks(&Value::Object(token));
                    }
                }
                Ok(Value::Null)
            }
            TOKEN_THROW_IF_CANCELLED => {
                if let Some(token) = args.iter().find_map(|v| match v {
                    Value::Object(o) if o.borrow().class_name == "CancellationToken" => Some(v.clone()),
                    _ => None,
                }) {
                    if token_is_cancelled(&token) {
                        return Err(RuntimeError::Message("operation cancelled".into()));
                    }
                }
                Ok(Value::Null)
            }
            TASKGROUP_NEW => Ok(crate::gc::alloc_object(ObjectInstance {
                class_name: "TaskGroup".into(),
                fields: HashMap::from([
                    ("tasks".into(), crate::gc::alloc_array(Vec::new())),
                    ("token".into(), Self::token_object(false)),
                ]),
                class_index: None,
                finalized: false,
            })),
            TASKGROUP_RUN => {
                let group = args
                    .iter()
                    .find(|v| matches!(v, Value::Object(o) if o.borrow().class_name == "TaskGroup"))
                    .cloned()
                    .ok_or_else(|| RuntimeError::Message("TaskGroup.Run requires a TaskGroup receiver".into()))?;
                let f = args
                    .iter()
                    .find_map(|v| match v {
                        Value::Function(f) => Some(f.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| RuntimeError::Message("TaskGroup.Run requires a function".into()))?;
                let token = Self::group_token(&group);
                let task = TaskInner::new_pending_with_token(token.clone());
                self.register_task_token(&task, token.as_ref());
                if token.as_ref().map(token_is_cancelled).unwrap_or(false) {
                    let _ = cancel_task(&task);
                } else {
                    self.spawn_coroutine(f.chunk_index, vec![], task.clone(), f.upvalues.clone());
                }
                Self::push_group_task(&group, Value::Task(task.clone()));
                Ok(Value::Task(task))
            }
            TASKGROUP_CANCEL => {
                let group = args
                    .iter()
                    .find(|v| matches!(v, Value::Object(o) if o.borrow().class_name == "TaskGroup"))
                    .cloned()
                    .ok_or_else(|| RuntimeError::Message("TaskGroup.Cancel requires a TaskGroup receiver".into()))?;
                if let Some(Value::Object(token)) = Self::group_token(&group) {
                    token
                        .borrow_mut()
                        .fields
                        .insert("isCancelled".into(), Value::Bool(true));
                }
                self.cancel_group_tasks(&group);
                self.poll_joins();
                Ok(Value::Null)
            }
            TASKGROUP_WHEN_ALL => {
                let group = args
                    .iter()
                    .find(|v| matches!(v, Value::Object(o) if o.borrow().class_name == "TaskGroup"))
                    .cloned()
                    .ok_or_else(|| RuntimeError::Message("TaskGroup.WhenAll requires a TaskGroup receiver".into()))?;
                let list = crate::gc::alloc_array(
                    Self::group_tasks(&group).into_iter().map(Value::Task).collect(),
                );
                self.create_join_task(&[list], false, Self::group_token(&group))
            }
            TASKGROUP_WHEN_ANY => {
                let group = args
                    .iter()
                    .find(|v| matches!(v, Value::Object(o) if o.borrow().class_name == "TaskGroup"))
                    .cloned()
                    .ok_or_else(|| RuntimeError::Message("TaskGroup.WhenAny requires a TaskGroup receiver".into()))?;
                let list = crate::gc::alloc_array(
                    Self::group_tasks(&group).into_iter().map(Value::Task).collect(),
                );
                self.create_join_task(&[list], true, Self::group_token(&group))
            }
            GC_COLLECT => {
                let before = self.heap.stats().objects_freed;
                let _ = self.gc_collect();
                let after = self.heap.stats().objects_freed;
                Ok(Value::Int((after - before) as i64))
            }
            GC_STATS => {
                let s = self.heap.stats();
                let mut map = HashMap::new();
                map.insert("collections".into(), Value::Int(s.collections as i64));
                map.insert("freed".into(), Value::Int(s.objects_freed as i64));
                map.insert("live".into(), Value::Int(s.live_objects as i64));
                map.insert("bytes".into(), Value::Int(s.live_bytes as i64));
                map.insert("enabled".into(), Value::Bool(s.enabled));
                Ok(crate::gc::alloc_dict(map))
            }
            TYPE_INVOKE => {
                let args = crate::stdlib::reflect::strip_type_receiver_pub(args);
                let obj = args
                    .first()
                    .cloned()
                    .ok_or_else(|| RuntimeError::Message("Type.Invoke(obj, name, …)".into()))?;
                let name = args
                    .get(1)
                    .map(|v| v.as_string())
                    .ok_or_else(|| {
                        RuntimeError::Message("Type.Invoke requires a method name".into())
                    })?;
                let f = crate::stdlib::reflect::find_method(&obj, &name)?;
                let mut call_args = vec![obj];
                if let Some(Value::Array(a)) = args.get(2) {
                    call_args.extend(a.borrow().iter().cloned());
                } else {
                    call_args.extend(args.iter().skip(2).cloned());
                }
                self.invoke_function(&f, &call_args)
            }
            _ => stdlib::call_native(id, args),
        }
    }
}

fn get_property(obj: &Value, name: &str) -> RuntimeResult<Value> {
    stdlib::get_property(obj, name)
}

fn set_property(obj: &mut Value, name: &str, value: Value) -> RuntimeResult<()> {
    match obj {
        Value::Object(o) => {
            o.borrow_mut().fields.insert(name.to_string(), value);
            Ok(())
        }
        _ => Err(RuntimeError::TypeError("cannot set property".into())),
    }
}

fn get_index(obj: &Value, index: &Value) -> RuntimeResult<Value> {
    match obj {
        Value::Array(a) => {
            let i = index.as_int()? as usize;
            a.borrow()
                .get(i)
                .cloned()
                .ok_or(RuntimeError::IndexOutOfRange)
        }
        Value::String(s) => {
            let i = index.as_int()? as usize;
            s.chars()
                .nth(i)
                .map(Value::Char)
                .ok_or(RuntimeError::IndexOutOfRange)
        }
        Value::Dict(d) => {
            let key = index.as_string();
            Ok(d.borrow().get(&key).cloned().unwrap_or(Value::Null))
        }
        Value::Object(o) => {
            let key = index.as_string();
            Ok(o.borrow()
                .fields
                .get(&key)
                .cloned()
                .unwrap_or(Value::Null))
        }
        _ => Err(RuntimeError::TypeError("value is not indexable".into())),
    }
}

fn set_index(obj: &mut Value, index: &Value, value: Value) -> RuntimeResult<()> {
    match obj {
        Value::Array(a) => {
            let i = index.as_int()? as usize;
            let mut arr = a.borrow_mut();
            if i >= arr.len() {
                arr.resize(i + 1, Value::Null);
            }
            arr[i] = value;
            Ok(())
        }
        Value::Dict(d) => {
            d.borrow_mut().insert(index.as_string(), value);
            Ok(())
        }
        Value::Object(o) => {
            o.borrow_mut()
                .fields
                .insert(index.as_string(), value);
            Ok(())
        }
        _ => Err(RuntimeError::TypeError("value is not indexable".into())),
    }
}

// ---- Debugger (DAP) support ------------------------------------------------

fn normalize_debug_path(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        s.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        s
    }
}

impl Vm {
    /// Prepare VM for debugging without running the event loop yet.
    pub fn debug_begin(
        &mut self,
        source: PathBuf,
        breakpoints: HashMap<String, Vec<DebugBreakpoint>>,
        pause_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.heap.install();
        let main = self.module.main_chunk;
        let local_count = self.module.chunks[main].local_count.max(8);
        self.stack.resize(local_count, Value::Null);
        self.frames.push(CallFrame {
            chunk: main,
            ip: 0,
            slots_start: 0,
            upvalues: vec![],
        });
        self.co_id = 0;
        self.co_task = None;
        let path_key = normalize_debug_path(&source);
        for chunk in &mut self.module.chunks {
            if chunk.source.is_none() {
                chunk.source = Some(source.display().to_string());
            }
        }
        self.debug = Some(VmDebug {
            source,
            breakpoints,
            last_line: None,
            last_path: Some(path_key),
            started: false,
            step_depth: 0,
            pause_flag,
        });
    }

    /// Remember the current source line so Continue does not re-stop on it.
    pub fn debug_mark_current_line(&mut self) {
        let line = self.debug_current_line();
        let path = self.debug_current_path();
        if let Some(d) = &mut self.debug {
            d.last_line = line;
            d.last_path = path;
        }
    }

    pub fn debug_set_breakpoints(&mut self, breakpoints: HashMap<String, Vec<DebugBreakpoint>>) {
        if let Some(d) = &mut self.debug {
            d.breakpoints = breakpoints;
        }
    }

    pub fn debug_current_line(&self) -> Option<usize> {
        let frame = self.frames.last()?;
        let chunk = self.module.chunks.get(frame.chunk)?;
        let ip = frame.ip.min(chunk.lines.len().saturating_sub(1));
        chunk.lines.get(ip).copied().filter(|&l| l > 0)
    }

    pub fn debug_current_path(&self) -> Option<String> {
        let frame = self.frames.last()?;
        let chunk = self.module.chunks.get(frame.chunk)?;
        chunk
            .source
            .as_ref()
            .map(|s| normalize_debug_path(std::path::Path::new(s)))
            .or_else(|| {
                self.debug
                    .as_ref()
                    .map(|d| normalize_debug_path(&d.source))
            })
    }

    pub fn debug_frame_count(&self) -> usize {
        self.frames.len()
    }

    fn debug_bp_hit(&self, path: &str, line: usize) -> Option<&DebugBreakpoint> {
        let dbg = self.debug.as_ref()?;
        let list = dbg.breakpoints.get(path).or_else(|| {
            // Fall back: match by file name only
            let file = std::path::Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())?;
            dbg.breakpoints.iter().find_map(|(k, v)| {
                let kn = std::path::Path::new(k)
                    .file_name()
                    .and_then(|s| s.to_str())?;
                if kn.eq_ignore_ascii_case(file) {
                    Some(v)
                } else {
                    None
                }
            })
        })?;
        list.iter().find(|b| b.line == line)
    }

    fn debug_condition_ok(&self, cond: &Option<String>) -> bool {
        match cond {
            None => true,
            Some(c) if c.trim().is_empty() => true,
            Some(c) => self.debug_eval_value(c.trim()).is_truthy(),
        }
    }

    /// Run until breakpoint, step boundary, pause, termination, or error.
    pub fn debug_run(
        &mut self,
        mode: crate::dap::VmStepMode,
    ) -> RuntimeResult<crate::dap::DebugStop> {
        use crate::dap::{DebugStop, VmStepMode};
        use std::sync::atomic::Ordering;

        let start_line = self.debug_current_line();
        let start_path = self.debug_current_path();
        let start_depth = self.frames.len();
        if let Some(d) = &mut self.debug {
            d.started = true;
            d.step_depth = start_depth;
            d.pause_flag.store(false, Ordering::SeqCst);
            if matches!(
                mode,
                VmStepMode::Next | VmStepMode::StepIn | VmStepMode::StepOut
            ) {
                d.last_line = start_line;
                d.last_path = start_path.clone();
            }
        }

        let mut ops = 0u32;
        loop {
            ops += 1;
            if ops % 64 == 0 {
                if let Some(d) = &self.debug {
                    if d.pause_flag.load(Ordering::SeqCst) {
                        let line = self.debug_current_line().unwrap_or(0);
                        return Ok(DebugStop::Pause { line });
                    }
                }
            }

            self.fire_timers();
            self.poll_joins();

            if self.frames.is_empty() {
                match self.resume_next() {
                    Ok(true) => {}
                    Ok(false) => {
                        if let Some(deadline) = self.timers.next_deadline() {
                            let now = Instant::now();
                            if deadline > now {
                                std::thread::sleep(
                                    (deadline - now).min(Duration::from_millis(50)),
                                );
                            }
                            continue;
                        }
                        let v = self.root_done.clone().unwrap_or(Value::Null);
                        GcHeap::uninstall();
                        return Ok(DebugStop::Terminated { result: v });
                    }
                    Err(e) => return Ok(DebugStop::Error(e)),
                }
                continue;
            }

            let line_before = self.debug_current_line();
            let path_before = self.debug_current_path();
            let depth = self.frames.len();

            if let (Some(line), Some(path)) = (line_before, path_before.clone()) {
                let bp_info = self.debug_bp_hit(&path, line).map(|b| {
                    (
                        b.condition.clone(),
                        b.log_message.clone(),
                    )
                });
                if let Some((cond, log_message)) = bp_info {
                    let same = self.debug.as_ref().map(|d| {
                        d.last_line == Some(line) && d.last_path.as_deref() == Some(path.as_str())
                    });
                    if same != Some(true) && self.debug_condition_ok(&cond) {
                        if let Some(msg) = log_message {
                            let text = self.debug_expand_logpoint(&msg);
                            crate::debug_io::write_stdout(&text);
                        } else {
                            if let Some(d) = &mut self.debug {
                                d.last_line = Some(line);
                                d.last_path = Some(path.clone());
                            }
                            return Ok(DebugStop::Breakpoint { line });
                        }
                    }
                }

                let step_stop = match mode {
                    VmStepMode::Next => {
                        depth <= start_depth
                            && (Some(line) != start_line || path_before != start_path)
                    }
                    VmStepMode::StepIn => {
                        Some(line) != start_line || path_before != start_path || depth > start_depth
                    }
                    VmStepMode::StepOut => depth < start_depth,
                    VmStepMode::Continue | VmStepMode::Pause => false,
                };
                if step_stop {
                    if let Some(d) = &mut self.debug {
                        d.last_line = Some(line);
                        d.last_path = Some(path);
                    }
                    return Ok(DebugStop::Step { line });
                }
            }

            let chunk_idx = self.frame().chunk;
            let ip = self.frame().ip;
            if ip >= self.module.chunks[chunk_idx].code.len() {
                let frame = self.frames.pop().expect("frame");
                self.stack.truncate(frame.slots_start);
                if self.frames.is_empty() {
                    match self.finish_coroutine(Value::Null) {
                        Ok(StepCtrl::Halt(v)) => {
                            GcHeap::uninstall();
                            return Ok(DebugStop::Terminated { result: v });
                        }
                        Ok(_) => continue,
                        Err(e) => return Ok(DebugStop::Error(e)),
                    }
                }
                self.push(Value::Null);
                continue;
            }

            let op = self.module.chunks[chunk_idx].code[ip];
            self.frame_mut().ip += 1;
            if let Some(d) = &mut self.debug {
                d.last_line = line_before;
                d.last_path = path_before.clone();
            }

            match self.step(op) {
                Ok(StepCtrl::Continue) => continue,
                Ok(StepCtrl::Halt(v)) => {
                    GcHeap::uninstall();
                    return Ok(DebugStop::Terminated { result: v });
                }
                Ok(StepCtrl::Yield) => continue,
                Err(e) => return Ok(DebugStop::Error(e)),
            }
        }
    }

    fn debug_expand_logpoint(&self, template: &str) -> String {
        // Replace {name} with evaluated names
        let mut out = String::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            rest = &rest[start + 1..];
            if let Some(end) = rest.find('}') {
                let name = &rest[..end];
                out.push_str(&self.debug_eval_name(name));
                rest = &rest[end + 1..];
            } else {
                out.push('{');
                break;
            }
        }
        out.push_str(rest);
        out
    }

    pub fn debug_stack_frames(&self) -> Vec<serde_json::Value> {
        let fallback = self
            .debug
            .as_ref()
            .map(|d| d.source.display().to_string())
            .unwrap_or_default();
        self.frames
            .iter()
            .rev()
            .enumerate()
            .map(|(i, frame)| {
                let chunk = self.module.chunks.get(frame.chunk);
                let name = chunk
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| "<chunk>".into());
                let line = chunk
                    .and_then(|c| c.lines.get(frame.ip.min(c.lines.len().saturating_sub(1))))
                    .copied()
                    .filter(|&l| l > 0)
                    .unwrap_or(1);
                let source_path = chunk
                    .and_then(|c| c.source.clone())
                    .unwrap_or_else(|| fallback.clone());
                let source_name = std::path::Path::new(&source_path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("main.rt");
                serde_json::json!({
                    "id": i,
                    "name": name,
                    "line": line,
                    "column": 1,
                    "source": {
                        "name": source_name,
                        "path": source_path
                    }
                })
            })
            .collect()
    }

    fn frame_by_dap_id(&self, frame_id: usize) -> Option<&CallFrame> {
        // DAP ids are reverse-enumerated
        let n = self.frames.len();
        if frame_id >= n {
            return None;
        }
        self.frames.get(n - 1 - frame_id)
    }

    pub fn debug_locals_for_frame(&self, frame_id: usize) -> Vec<serde_json::Value> {
        let Some(frame) = self.frame_by_dap_id(frame_id) else {
            return vec![];
        };
        let chunk = match self.module.chunks.get(frame.chunk) {
            Some(c) => c,
            None => return vec![],
        };
        let ip = frame.ip;
        let start = frame.slots_start;
        let mut seen = HashSet::new();
        let mut vars = Vec::new();

        for ld in &chunk.local_debug {
            if ip >= ld.start_ip && ip < ld.end_ip {
                if !seen.insert(ld.name.clone()) {
                    continue; // shadowing: keep innermost (ranges listed chronologically; last wins — reverse)
                }
                let idx = start + ld.slot as usize;
                let v = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                vars.push(self.debug_value_json(&ld.name, &v, 1000 + ld.slot as i64));
            }
        }
        // Prefer innermost shadows: rebuild keeping last occurrence of each name
        let mut by_name: HashMap<String, serde_json::Value> = HashMap::new();
        for v in vars {
            if let Some(n) = v.get("name").and_then(|x| x.as_str()) {
                by_name.insert(n.to_string(), v);
            }
        }
        let mut out: Vec<_> = by_name.into_values().collect();
        out.sort_by(|a, b| {
            a.get("name")
                .and_then(|x| x.as_str())
                .cmp(&b.get("name").and_then(|x| x.as_str()))
        });
        if out.is_empty() {
            // Fallback unnamed slots
            for i in 0..chunk.local_count.max(8) {
                let idx = start + i;
                if idx >= self.stack.len() {
                    break;
                }
                let v = &self.stack[idx];
                out.push(self.debug_value_json(&format!("local_{}", i), v, 1000 + i as i64));
            }
        }
        out
    }

    pub fn debug_locals(&self) -> Vec<serde_json::Value> {
        self.debug_locals_for_frame(0)
    }

    pub fn debug_globals(&self) -> Vec<serde_json::Value> {
        self.module
            .globals
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let v = self.globals.get(i).cloned().unwrap_or(Value::Null);
                self.debug_value_json(name, &v, 2000 + i as i64)
            })
            .collect()
    }

    fn debug_value_json(&self, name: &str, v: &Value, child_base: i64) -> serde_json::Value {
        let expandable = matches!(v, Value::Array(_) | Value::Dict(_) | Value::Object(_));
        serde_json::json!({
            "name": name,
            "value": v.as_string(),
            "type": v.type_name(),
            "variablesReference": if expandable { child_base } else { 0 }
        })
    }

    /// Expand variablesReference from locals (1xxx) / globals (2xxx).
    pub fn debug_expand_var(&self, variables_ref: i64) -> Vec<serde_json::Value> {
        if (1000..2000).contains(&variables_ref) {
            let slot = (variables_ref - 1000) as usize;
            let Some(frame) = self.frames.last() else {
                return vec![];
            };
            let idx = frame.slots_start + slot;
            let Some(v) = self.stack.get(idx) else {
                return vec![];
            };
            return self.debug_children(v);
        }
        if (2000..3000_i64).contains(&variables_ref) {
            let gi = (variables_ref - 2000) as usize;
            let Some(v) = self.globals.get(gi) else {
                return vec![];
            };
            return self.debug_children(v);
        }
        return vec![];
    }

    fn debug_children(&self, v: &Value) -> Vec<serde_json::Value> {
        match v {
            Value::Array(a) => a
                .borrow()
                .iter()
                .enumerate()
                .map(|(i, el)| {
                    serde_json::json!({
                        "name": format!("[{}]", i),
                        "value": el.as_string(),
                        "type": el.type_name(),
                        "variablesReference": 0
                    })
                })
                .collect(),
            Value::Dict(d) => d
                .borrow()
                .iter()
                .map(|(k, el)| {
                    serde_json::json!({
                        "name": k,
                        "value": el.as_string(),
                        "type": el.type_name(),
                        "variablesReference": 0
                    })
                })
                .collect(),
            Value::Object(o) => {
                let obj = o.borrow();
                let mut kids = vec![serde_json::json!({
                    "name": "__class",
                    "value": obj.class_name,
                    "type": "string",
                    "variablesReference": 0
                })];
                let mut fields: Vec<_> = obj.fields.iter().collect();
                fields.sort_by(|a, b| a.0.cmp(b.0));
                for (k, el) in fields {
                    kids.push(serde_json::json!({
                        "name": k,
                        "value": el.as_string(),
                        "type": el.type_name(),
                        "variablesReference": 0
                    }));
                }
                kids
            }
            _ => vec![],
        }
    }

    pub fn debug_eval_value(&self, name: &str) -> Value {
        if let Some(idx) = self.module.globals.iter().position(|g| g == name) {
            return self.globals.get(idx).cloned().unwrap_or(Value::Null);
        }
        if let Some(frame) = self.frames.last() {
            if let Some(rest) = name.strip_prefix("local_") {
                if let Ok(i) = rest.parse::<usize>() {
                    let idx = frame.slots_start + i;
                    if idx < self.stack.len() {
                        return self.stack[idx].clone();
                    }
                }
            }
            if let Some(chunk) = self.module.chunks.get(frame.chunk) {
                let ip = frame.ip;
                for ld in chunk.local_debug.iter().rev() {
                    if ld.name == name && ip >= ld.start_ip && ip < ld.end_ip {
                        let idx = frame.slots_start + ld.slot as usize;
                        if idx < self.stack.len() {
                            return self.stack[idx].clone();
                        }
                    }
                }
            }
        }
        Value::Null
    }

    pub fn debug_eval_name(&self, name: &str) -> String {
        let v = self.debug_eval_value(name);
        if matches!(v, Value::Null) && !self.module.globals.iter().any(|g| g == name) {
            // distinguish unknown
            let known_local = self.frames.last().and_then(|frame| {
                self.module.chunks.get(frame.chunk).map(|c| {
                    c.local_debug.iter().any(|ld| {
                        ld.name == name && frame.ip >= ld.start_ip && frame.ip < ld.end_ip
                    })
                })
            });
            if known_local != Some(true) && name.strip_prefix("local_").is_none() {
                return format!("<unknown {}>", name);
            }
        }
        v.as_string()
    }

    pub fn debug_request_pause(&self) {
        if let Some(d) = &self.debug {
            d.pause_flag
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}


