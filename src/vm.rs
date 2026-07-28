//! Bytecode virtual machine with async/await event-loop.

use crate::async_rt::{
    add_waiter, complete_task, fail_task, ReadyQueue, TaskHandle, TaskInner, TimerQueue,
};
use crate::bytecode::{Module, Op};
use crate::error::{RuntimeError, RuntimeResult};
use crate::gc::{GcConfig, GcHeap, GcStats};
use crate::stdlib;
use crate::value::{binary_op, FunctionRef, ObjectInstance, UpvalueCell, Value};
use std::cell::RefCell;
use std::collections::HashMap;
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
struct JoinAll {
    outer: TaskHandle,
    tasks: Vec<TaskHandle>,
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
    try_stack: Vec<TryFrame>,
    /// Current coroutine id.
    co_id: usize,
    co_task: Option<TaskHandle>,
    parked: HashMap<usize, ParkedCo>,
    ready: ReadyQueue,
    timers: TimerQueue,
    joins: Vec<JoinAll>,
    next_co_id: usize,
    /// >0 while running nested sync invoke (LINQ etc.) — await not allowed.
    sync_depth: usize,
    root_done: Option<Value>,
    heap: GcHeap,
}

impl Vm {
    pub fn new(module: Module) -> Self {
        Self::with_gc(module, GcConfig::default())
    }

    pub fn with_gc(module: Module, gc: GcConfig) -> Self {
        let n = module.globals.len();
        let mut globals = vec![Value::Null; n.max(64)];
        for (i, name) in module.globals.iter().enumerate() {
            if let Some(v) = stdlib::builtin_global(name) {
                globals[i] = v;
            }
        }
        Self {
            module,
            stack: Vec::with_capacity(256),
            frames: Vec::new(),
            globals,
            try_stack: Vec::new(),
            co_id: 0,
            co_task: None,
            parked: HashMap::new(),
            ready: ReadyQueue::new(),
            timers: TimerQueue::new(),
            joins: Vec::new(),
            next_co_id: 1,
            sync_depth: 0,
            root_done: None,
            heap: GcHeap::new(gc),
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
        let should = self.heap.config().stress
            || self.heap.stats().live_bytes >= self.heap.config().threshold_bytes;
        // Use allocated_bytes via maybe_collect
        let roots = self.collect_roots();
        self.heap.maybe_collect(&roots);
        let finals = self.heap.take_pending_finalizers();
        for obj in finals {
            let _ = self.run_finalizer(&obj);
        }
        let _ = should;
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
                        self.push(Value::String(format!("{}", e).into()));
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
            let waiters = complete_task(&task, result);
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
        for (_task, waiters) in fired {
            for w in waiters {
                self.ready.push(w);
            }
        }
    }

    fn poll_joins(&mut self) {
        let mut still = Vec::new();
        for join in self.joins.drain(..) {
            let all_ready = join.tasks.iter().all(|t| t.borrow().is_ready());
            if all_ready {
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
                if matches!(v, Value::Null) {
                    if let Some(name) = self.module.globals.get(idx) {
                        if let Some(b) = stdlib::builtin_global(name) {
                            self.globals[idx] = b.clone();
                            self.push(b);
                            return Ok(StepCtrl::Continue);
                        }
                    }
                }
                self.push(v);
            }
            x if x == Op::SetGlobal as u8 => {
                let idx = self.read_byte() as usize;
                let v = self.peek(0)?.clone();
                if idx >= self.globals.len() {
                    self.globals.resize(idx + 1, Value::Null);
                }
                self.globals[idx] = v;
            }
            x if x == Op::DefineGlobal as u8 => {
                let idx = self.read_byte() as usize;
                let v = self.pop()?;
                if idx >= self.globals.len() {
                    self.globals.resize(idx + 1, Value::Null);
                }
                self.globals[idx] = v;
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
                            Value::String(_) => "string".into(),
                            Value::Array(_) => "List".into(),
                            Value::Int(_) => "int".into(),
                            Value::Float(_) => "double".into(),
                            Value::Bool(_) => "bool".into(),
                            other => other.type_name().to_string(),
                        };
                        let key = format!("{}.{}", type_name, name);
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
                println!("{}", v.as_string());
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
            other => {
                return Err(RuntimeError::Message(format!("unknown opcode {}", other)));
            }
        }
        Ok(StepCtrl::Continue)
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
                        | crate::stdlib::ids::THREAD_RUN
                        | crate::stdlib::ids::GC_COLLECT
                        | crate::stdlib::ids::GC_STATS
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
                let task = TaskInner::new_pending();
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
                let task = TaskInner::new_pending();
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
                let list = args
                    .iter()
                    .find_map(|v| match v {
                        Value::Array(a) => Some(a.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        RuntimeError::Message("Task.WhenAll requires an array of Tasks".into())
                    })?;
                let items: Vec<Value> = list.borrow().clone();
                let mut tasks = Vec::new();
                for v in items {
                    match v {
                        Value::Task(t) => tasks.push(t),
                        other => {
                            // Wrap non-task as already-ready
                            tasks.push(TaskInner::new_ready(other));
                        }
                    }
                }
                let outer = TaskInner::new_pending();
                if tasks.is_empty() {
                    let _ = complete_task(
                        &outer,
                        crate::gc::alloc_array(Vec::new()),
                    );
                    return Ok(Value::Task(outer));
                }
                self.joins.push(JoinAll {
                    outer: outer.clone(),
                    tasks,
                });
                self.poll_joins();
                Ok(Value::Task(outer))
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
