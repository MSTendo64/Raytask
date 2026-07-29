//! Bytecode instruction set and chunks.

use crate::value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Op {
    Constant = 1,
    Null,
    True,
    False,
    Pop,
    GetLocal,
    SetLocal,
    GetGlobal,
    SetGlobal,
    DefineGlobal,
    GetProperty,
    SetProperty,
    GetIndex,
    SetIndex,
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
    Neg,
    Not,
    BitNot,
    NullCoalesce,
    Jump,
    JumpIfFalse,
    JumpIfTrue,
    Loop,
    Call,
    Return,
    NewObject,
    NewArray,
    Print,
    Dup,
    Halt,
    Throw,
    TryBegin,
    TryEnd,
    IsNull,
    ToString,
    IncLocal,
    DecLocal,
    /// Pop function proto; operand n + n×(is_local, index) capture descriptors.
    /// Pushes Function with upvalues filled from current frame.
    MakeClosure,
    /// Await a Task on the stack; suspends coroutine if pending.
    Await,
    /// Read closed-over value (operand = upvalue index).
    GetUpvalue,
    /// Write closed-over value (operand = upvalue index); leaves value on stack.
    SetUpvalue,
}

#[derive(Debug, Clone)]
pub struct LocalDebug {
    pub name: String,
    pub slot: u8,
    /// Inclusive start IP in this chunk's code.
    pub start_ip: usize,
    /// Exclusive end IP; `usize::MAX` while the local is still in scope at end of compile.
    pub end_ip: usize,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub name: String,
    pub code: Vec<u8>,
    pub constants: Vec<Value>,
    pub lines: Vec<usize>,
    pub arity: usize,
    pub local_count: usize,
    /// If true, calling this chunk returns a Task and runs as a coroutine.
    pub is_async: bool,
    /// Live ranges for debugger variable display.
    pub local_debug: Vec<LocalDebug>,
    /// Absolute or relative source path for this chunk (debug).
    pub source: Option<String>,
}

impl Chunk {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            code: Vec::new(),
            constants: Vec::new(),
            lines: Vec::new(),
            arity: 0,
            local_count: 0,
            is_async: false,
            local_debug: Vec::new(),
            source: None,
        }
    }

    pub fn emit_op(&mut self, op: Op, line: usize) {
        self.code.push(op as u8);
        self.lines.push(line);
    }

    pub fn emit_byte(&mut self, byte: u8, line: usize) {
        self.code.push(byte);
        self.lines.push(line);
    }

    pub fn emit_u16(&mut self, value: u16, line: usize) {
        self.emit_byte((value >> 8) as u8, line);
        self.emit_byte((value & 0xff) as u8, line);
    }

    pub fn emit_jump(&mut self, op: Op, line: usize) -> usize {
        self.emit_op(op, line);
        self.emit_byte(0xff, line);
        self.emit_byte(0xff, line);
        self.code.len() - 2
    }

    pub fn patch_jump(&mut self, offset: usize) {
        let jump = self.code.len() - offset - 2;
        if jump > u16::MAX as usize {
            panic!("jump too large");
        }
        self.code[offset] = ((jump >> 8) & 0xff) as u8;
        self.code[offset + 1] = (jump & 0xff) as u8;
    }

    pub fn emit_loop(&mut self, loop_start: usize, line: usize) {
        self.emit_op(Op::Loop, line);
        let jump = self.code.len() - loop_start + 2;
        self.emit_u16(jump as u16, line);
    }

    pub fn add_constant(&mut self, value: Value) -> u8 {
        // dedup simple constants
        for (i, c) in self.constants.iter().enumerate() {
            if c == &value {
                return i as u8;
            }
        }
        let idx = self.constants.len();
        if idx > 255 {
            panic!("too many constants");
        }
        self.constants.push(value);
        idx as u8
    }

    pub fn emit_constant(&mut self, value: Value, line: usize) {
        let idx = self.add_constant(value);
        self.emit_op(Op::Constant, line);
        self.emit_byte(idx, line);
    }

    pub fn read_u16(&self, ip: usize) -> u16 {
        ((self.code[ip] as u16) << 8) | (self.code[ip + 1] as u16)
    }
}

#[derive(Debug, Clone)]
pub struct Module {
    pub chunks: Vec<Chunk>,
    pub main_chunk: usize,
    pub globals: Vec<String>,
    pub classes: Vec<ClassInfo>,
    pub ffi: crate::ffi::FfiModuleInfo,
    pub stdlib_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub name: String,
    pub fields: Vec<String>,
    pub methods: Vec<(String, usize)>, // name -> chunk index
    pub constructor: Option<usize>,
    /// Index of base class in Module.classes, if any.
    pub base: Option<usize>,
    /// Chunk index of destructor (`~new`), if any.
    pub destructor: Option<usize>,
}
