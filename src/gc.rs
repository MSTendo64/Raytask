//! Mark-sweep GC with Rc-backed heap objects (Array / Dict / Object).

use crate::value::{ObjectInstance, Value};
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    static CURRENT: RefCell<Option<*mut GcHeap>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GcId(pub u32);

#[derive(Debug, Clone)]
pub struct GcConfig {
    pub enabled: bool,
    pub threshold_bytes: usize,
    pub stress: bool,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_bytes: 256 * 1024,
            stress: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GcStats {
    pub collections: u64,
    pub objects_freed: u64,
    pub bytes_freed: u64,
    pub live_objects: usize,
    pub live_bytes: usize,
    pub enabled: bool,
}

#[derive(Debug)]
pub struct GcArray {
    pub marked: Cell<bool>,
    pub items: RefCell<Vec<Value>>,
    id: GcId,
}

impl GcArray {
    pub fn borrow(&self) -> Ref<'_, Vec<Value>> {
        self.items.borrow()
    }
    pub fn borrow_mut(&self) -> RefMut<'_, Vec<Value>> {
        self.items.borrow_mut()
    }
    pub fn id(&self) -> GcId {
        self.id
    }
}

#[derive(Debug)]
pub struct GcDict {
    pub marked: Cell<bool>,
    pub map: RefCell<HashMap<String, Value>>,
    id: GcId,
}

impl GcDict {
    pub fn borrow(&self) -> Ref<'_, HashMap<String, Value>> {
        self.map.borrow()
    }
    pub fn borrow_mut(&self) -> RefMut<'_, HashMap<String, Value>> {
        self.map.borrow_mut()
    }
    pub fn id(&self) -> GcId {
        self.id
    }
}

#[derive(Debug)]
pub struct GcObject {
    pub marked: Cell<bool>,
    pub data: RefCell<ObjectInstance>,
    id: GcId,
}

impl GcObject {
    pub fn borrow(&self) -> Ref<'_, ObjectInstance> {
        self.data.borrow()
    }
    pub fn borrow_mut(&self) -> RefMut<'_, ObjectInstance> {
        self.data.borrow_mut()
    }
    pub fn id(&self) -> GcId {
        self.id
    }
}

enum Tracked {
    Array(Rc<GcArray>),
    Dict(Rc<GcDict>),
    Object(Rc<GcObject>),
}

pub struct GcHeap {
    config: GcConfig,
    tracked: Vec<Tracked>,
    next_id: u32,
    allocated_bytes: usize,
    collections: u64,
    objects_freed: u64,
    bytes_freed: u64,
    pending_finalizers: Vec<Rc<GcObject>>,
}

impl GcHeap {
    pub fn new(config: GcConfig) -> Self {
        Self {
            config,
            tracked: Vec::new(),
            next_id: 1,
            allocated_bytes: 0,
            collections: 0,
            objects_freed: 0,
            bytes_freed: 0,
            pending_finalizers: Vec::new(),
        }
    }

    pub fn config(&self) -> &GcConfig {
        &self.config
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
    }

    pub fn stats(&self) -> GcStats {
        GcStats {
            collections: self.collections,
            objects_freed: self.objects_freed,
            bytes_freed: self.bytes_freed,
            live_objects: self.tracked.len(),
            live_bytes: self.allocated_bytes,
            enabled: self.config.enabled,
        }
    }

    pub fn install(&mut self) {
        CURRENT.with(|c| *c.borrow_mut() = Some(self as *mut GcHeap));
    }

    pub fn uninstall() {
        CURRENT.with(|c| *c.borrow_mut() = None);
    }

    fn next_id(&mut self) -> GcId {
        let id = GcId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    pub fn alloc_array(&mut self, items: Vec<Value>) -> Rc<GcArray> {
        let bytes = 32 + items.capacity() * std::mem::size_of::<Value>();
        let id = self.next_id();
        let rc = Rc::new(GcArray {
            marked: Cell::new(false),
            items: RefCell::new(items),
            id,
        });
        self.tracked.push(Tracked::Array(rc.clone()));
        self.allocated_bytes += bytes;
        rc
    }

    pub fn alloc_dict(&mut self, map: HashMap<String, Value>) -> Rc<GcDict> {
        let bytes = 32 + map.capacity() * 32;
        let id = self.next_id();
        let rc = Rc::new(GcDict {
            marked: Cell::new(false),
            map: RefCell::new(map),
            id,
        });
        self.tracked.push(Tracked::Dict(rc.clone()));
        self.allocated_bytes += bytes;
        rc
    }

    pub fn alloc_object(&mut self, obj: ObjectInstance) -> Rc<GcObject> {
        let bytes = 48 + obj.fields.len() * 32;
        let id = self.next_id();
        let rc = Rc::new(GcObject {
            marked: Cell::new(false),
            data: RefCell::new(obj),
            id,
        });
        self.tracked.push(Tracked::Object(rc.clone()));
        self.allocated_bytes += bytes;
        rc
    }

    pub fn maybe_collect(&mut self, roots: &[Value]) {
        if !self.config.enabled {
            return;
        }
        if self.config.stress || self.allocated_bytes >= self.config.threshold_bytes {
            self.collect(roots);
        }
    }

    pub fn collect(&mut self, roots: &[Value]) -> GcStats {
        if !self.config.enabled {
            return self.stats();
        }
        self.collections += 1;

        for t in &self.tracked {
            match t {
                Tracked::Array(r) => r.marked.set(false),
                Tracked::Dict(r) => r.marked.set(false),
                Tracked::Object(r) => r.marked.set(false),
            }
        }

        let mut stack: Vec<Value> = roots.to_vec();
        while let Some(v) = stack.pop() {
            match v {
                Value::Array(a) => {
                    if !a.marked.replace(true) {
                        stack.extend(a.items.borrow().iter().cloned());
                    }
                }
                Value::Dict(d) => {
                    if !d.marked.replace(true) {
                        stack.extend(d.map.borrow().values().cloned());
                    }
                }
                Value::Object(o) => {
                    if !o.marked.replace(true) {
                        stack.extend(o.data.borrow().fields.values().cloned());
                    }
                }
                Value::Task(t) => {
                    if let crate::async_rt::TaskState::Ready(inner) = &t.borrow().state {
                        stack.push(inner.clone());
                    }
                }
                Value::Function(f) => {
                    for uv in &f.upvalues {
                        stack.push(uv.borrow().clone());
                    }
                }
                _ => {}
            }
        }

        let mut finals = Vec::new();
        for t in &self.tracked {
            if let Tracked::Object(o) = t {
                if !o.marked.get() && !o.data.borrow().finalized {
                    finals.push(o.clone());
                }
            }
        }
        self.pending_finalizers = finals;

        let mut freed = 0u64;
        let mut freed_bytes = 0u64;
        let mut alive = Vec::with_capacity(self.tracked.len());
        for t in self.tracked.drain(..) {
            let keep = match &t {
                Tracked::Array(r) => {
                    if r.marked.get() {
                        true
                    } else {
                        let n = r.items.borrow().len();
                        r.items.borrow_mut().clear();
                        freed += 1;
                        freed_bytes += (32 + n * 16) as u64;
                        false
                    }
                }
                Tracked::Dict(r) => {
                    if r.marked.get() {
                        true
                    } else {
                        let n = r.map.borrow().len();
                        r.map.borrow_mut().clear();
                        freed += 1;
                        freed_bytes += (32 + n * 32) as u64;
                        false
                    }
                }
                Tracked::Object(r) => {
                    if r.marked.get() {
                        true
                    } else {
                        let n = r.data.borrow().fields.len();
                        r.data.borrow_mut().fields.clear();
                        freed += 1;
                        freed_bytes += (48 + n * 32) as u64;
                        false
                    }
                }
            };
            if keep {
                alive.push(t);
            }
        }
        self.tracked = alive;
        self.objects_freed += freed;
        self.bytes_freed += freed_bytes;
        self.allocated_bytes = self.allocated_bytes.saturating_sub(freed_bytes as usize);

        if self.config.threshold_bytes < 16 * 1024 * 1024 {
            self.config.threshold_bytes = (self.allocated_bytes.saturating_mul(2)).max(64 * 1024);
        }

        self.stats()
    }

    pub fn take_pending_finalizers(&mut self) -> Vec<Rc<GcObject>> {
        std::mem::take(&mut self.pending_finalizers)
    }
}

thread_local! {
    static FALLBACK: RefCell<GcHeap> = RefCell::new(GcHeap::new(GcConfig::default()));
}

pub fn with_heap_mut<R>(f: impl FnOnce(&mut GcHeap) -> R) -> R {
    let existing = CURRENT.with(|c| *c.borrow());
    if let Some(p) = existing {
        return f(unsafe { &mut *p });
    }
    FALLBACK.with(|h| {
        let mut heap = h.borrow_mut();
        let p = &mut *heap as *mut GcHeap;
        CURRENT.with(|c| *c.borrow_mut() = Some(p));
        let r = f(&mut *heap);
        CURRENT.with(|c| *c.borrow_mut() = None);
        r
    })
}

pub fn alloc_array(items: Vec<Value>) -> Value {
    Value::Array(with_heap_mut(|h| h.alloc_array(items)))
}

pub fn alloc_dict(map: HashMap<String, Value>) -> Value {
    Value::Dict(with_heap_mut(|h| h.alloc_dict(map)))
}

pub fn alloc_object(obj: ObjectInstance) -> Value {
    Value::Object(with_heap_mut(|h| h.alloc_object(obj)))
}
