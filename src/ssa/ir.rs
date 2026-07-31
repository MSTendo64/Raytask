//! Mid-level SSA IR for RayTask optimizations.

use crate::value::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuncId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsaTy {
    Void,
    Bool,
    Int,
    Float,
    Ref,
    Dyn,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    /// Index into module function table / native id encoded later.
    FuncRef(FuncId),
    Native(usize),
    TypeModule(String),
}

#[derive(Debug, Clone)]
pub enum BinOpKind {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    NullCoalesce,
}

#[derive(Debug, Clone)]
pub enum UnOpKind {
    Neg,
    Not,
    BitNot,
    IsNull,
    ToString,
}

#[derive(Debug, Clone)]
pub enum InstKind {
    Const(ConstValue),
    /// Stack slot / local alloca (promoted by mem2reg).
    Alloca {
        ty: SsaTy,
        slot: u32,
    },
    Load {
        ptr: ValueId,
    },
    Store {
        ptr: ValueId,
        value: ValueId,
    },
    BinOp {
        op: BinOpKind,
        lhs: ValueId,
        rhs: ValueId,
    },
    UnOp {
        op: UnOpKind,
        arg: ValueId,
    },
    Call {
        callee: ValueId,
        args: Vec<ValueId>,
        effectful: bool,
    },
    GetGlobal {
        index: u32,
    },
    SetGlobal {
        index: u32,
        value: ValueId,
    },
    DefineGlobal {
        index: u32,
        value: ValueId,
    },
    GetProperty {
        object: ValueId,
        name: ValueId,
    },
    SetProperty {
        object: ValueId,
        name: ValueId,
        value: ValueId,
    },
    GetIndex {
        object: ValueId,
        index: ValueId,
    },
    SetIndex {
        object: ValueId,
        index: ValueId,
        value: ValueId,
    },
    NewObject {
        class_index: u8,
        name: Option<ValueId>,
    },
    NewArray {
        elems: Vec<ValueId>,
    },
    Print {
        value: ValueId,
    },
    Dup {
        value: ValueId,
    },
    Await {
        value: ValueId,
    },
    GetUpvalue {
        index: u8,
    },
    SetUpvalue {
        index: u8,
        value: ValueId,
    },
    MakeClosure {
        proto: ValueId,
        captures: Vec<(bool, u8)>,
    },
    Phi {
        incomings: Vec<(BlockId, ValueId)>,
    },
    /// Parameter / arg slot.
    Param {
        index: u32,
    },
}

#[derive(Debug, Clone)]
pub struct Inst {
    pub id: ValueId,
    pub kind: InstKind,
    pub ty: SsaTy,
    pub line: usize,
    /// Side effects that block DCE / speculative motion.
    pub effectful: bool,
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Br(BlockId),
    CondBr {
        cond: ValueId,
        then_bb: BlockId,
        else_bb: BlockId,
    },
    Return(Option<ValueId>),
    Halt,
    Throw(ValueId),
    Unreachable,
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub insts: Vec<Inst>,
    pub term: Terminator,
    pub preds: Vec<BlockId>,
    pub succs: Vec<BlockId>,
}

impl BasicBlock {
    pub fn new(id: BlockId) -> Self {
        Self {
            id,
            insts: Vec::new(),
            term: Terminator::Unreachable,
            preds: Vec::new(),
            succs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SsaFunction {
    pub id: FuncId,
    pub name: String,
    pub arity: usize,
    pub local_count: usize,
    pub is_async: bool,
    pub entry: BlockId,
    pub blocks: HashMap<BlockId, BasicBlock>,
    pub next_value: u32,
    pub next_block: u32,
    pub source: Option<String>,
}

impl SsaFunction {
    pub fn new(id: FuncId, name: impl Into<String>, arity: usize) -> Self {
        let entry = BlockId(0);
        let mut blocks = HashMap::new();
        blocks.insert(entry, BasicBlock::new(entry));
        Self {
            id,
            name: name.into(),
            arity,
            local_count: arity,
            is_async: false,
            entry,
            blocks,
            next_value: 0,
            next_block: 1,
            source: None,
        }
    }

    pub fn alloc_value(&mut self) -> ValueId {
        let id = ValueId(self.next_value);
        self.next_value += 1;
        id
    }

    pub fn create_block(&mut self) -> BlockId {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        self.blocks.insert(id, BasicBlock::new(id));
        id
    }

    pub fn block(&self, id: BlockId) -> &BasicBlock {
        &self.blocks[&id]
    }

    pub fn block_mut(&mut self, id: BlockId) -> &mut BasicBlock {
        self.blocks.get_mut(&id).expect("block")
    }

    pub fn push_inst(&mut self, bb: BlockId, mut inst: Inst) -> ValueId {
        if matches!(
            inst.kind,
            InstKind::Store { .. }
                | InstKind::Call {
                    effectful: true,
                    ..
                }
                | InstKind::SetGlobal { .. }
                | InstKind::DefineGlobal { .. }
                | InstKind::SetProperty { .. }
                | InstKind::SetIndex { .. }
                | InstKind::Print { .. }
                | InstKind::Await { .. }
                | InstKind::SetUpvalue { .. }
                | InstKind::NewObject { .. }
                | InstKind::NewArray { .. }
                | InstKind::MakeClosure { .. }
        ) {
            inst.effectful = true;
        }
        let id = inst.id;
        self.block_mut(bb).insts.push(inst);
        id
    }

    pub fn set_term(&mut self, bb: BlockId, term: Terminator) {
        self.block_mut(bb).term = term;
    }

    pub fn values(&self) -> impl Iterator<Item = (&BlockId, &Inst)> {
        self.blocks
            .iter()
            .flat_map(|(bid, b)| b.insts.iter().map(move |i| (bid, i)))
    }

    pub fn find_def(&self, id: ValueId) -> Option<&Inst> {
        for b in self.blocks.values() {
            if let Some(i) = b.insts.iter().find(|i| i.id == id) {
                return Some(i);
            }
        }
        None
    }

    pub fn replace_uses(&mut self, from: ValueId, to: ValueId) {
        for b in self.blocks.values_mut() {
            for inst in &mut b.insts {
                rewrite_inst_uses(&mut inst.kind, from, to);
            }
            match &mut b.term {
                Terminator::CondBr { cond, .. } if *cond == from => *cond = to,
                Terminator::Return(Some(v)) if *v == from => *v = to,
                Terminator::Throw(v) if *v == from => *v = to,
                _ => {}
            }
        }
    }

    pub fn remove_inst(&mut self, bb: BlockId, vid: ValueId) {
        let b = self.block_mut(bb);
        b.insts.retain(|i| i.id != vid);
    }
}

fn rewrite_inst_uses(kind: &mut InstKind, from: ValueId, to: ValueId) {
    let rewrite = |v: &mut ValueId| {
        if *v == from {
            *v = to;
        }
    };
    match kind {
        InstKind::Load { ptr } => rewrite(ptr),
        InstKind::Store { ptr, value } => {
            rewrite(ptr);
            rewrite(value);
        }
        InstKind::BinOp { lhs, rhs, .. } => {
            rewrite(lhs);
            rewrite(rhs);
        }
        InstKind::UnOp { arg, .. } => rewrite(arg),
        InstKind::Call { callee, args, .. } => {
            rewrite(callee);
            for a in args {
                rewrite(a);
            }
        }
        InstKind::SetGlobal { value, .. } => rewrite(value),
        InstKind::DefineGlobal { value, .. } => rewrite(value),
        InstKind::GetProperty { object, name } => {
            rewrite(object);
            rewrite(name);
        }
        InstKind::SetProperty {
            object,
            name,
            value,
        } => {
            rewrite(object);
            rewrite(name);
            rewrite(value);
        }
        InstKind::GetIndex { object, index } => {
            rewrite(object);
            rewrite(index);
        }
        InstKind::SetIndex {
            object,
            index,
            value,
        } => {
            rewrite(object);
            rewrite(index);
            rewrite(value);
        }
        InstKind::NewObject { name: Some(n), .. } => rewrite(n),
        InstKind::NewArray { elems } => {
            for e in elems {
                rewrite(e);
            }
        }
        InstKind::Print { value } => rewrite(value),
        InstKind::Dup { value } => rewrite(value),
        InstKind::Await { value } => rewrite(value),
        InstKind::SetUpvalue { value, .. } => rewrite(value),
        InstKind::MakeClosure { proto, .. } => rewrite(proto),
        InstKind::Phi { incomings } => {
            for (_, v) in incomings {
                rewrite(v);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone)]
pub struct SsaClass {
    pub name: String,
    pub fields: Vec<String>,
    pub methods: Vec<(String, FuncId)>,
    pub constructor: Option<FuncId>,
    pub base: Option<usize>,
    pub destructor: Option<FuncId>,
}

#[derive(Debug, Clone)]
pub struct SsaModule {
    pub functions: Vec<SsaFunction>,
    pub main_chunk: FuncId,
    pub globals: Vec<String>,
    pub classes: Vec<SsaClass>,
    pub ffi: crate::ffi::FfiModuleInfo,
    pub stdlib_enabled: bool,
    /// Constant pool helpers carried from bytecode lift (string/function constants).
    pub const_pool: Vec<Value>,
}

impl SsaModule {
    pub fn func(&self, id: FuncId) -> &SsaFunction {
        &self.functions[id.0 as usize]
    }

    pub fn func_mut(&mut self, id: FuncId) -> &mut SsaFunction {
        &mut self.functions[id.0 as usize]
    }
}
