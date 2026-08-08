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
    /// Pop Type, pop value; push bool — inheritance / primitive check.
    IsInstance,
    /// Constant with u16 index (when pool exceeds 255).
    Constant16,
    /// Global ops with u16 index (when globals exceed 255).
    GetGlobal16,
    SetGlobal16,
    DefineGlobal16,
    /// Pops prefix, peeks at string on stack; pushes bool: whether string starts with prefix.
    StringStartsWith,
}

impl Op {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => Self::Constant,
            2 => Self::Null,
            3 => Self::True,
            4 => Self::False,
            5 => Self::Pop,
            6 => Self::GetLocal,
            7 => Self::SetLocal,
            8 => Self::GetGlobal,
            9 => Self::SetGlobal,
            10 => Self::DefineGlobal,
            11 => Self::GetProperty,
            12 => Self::SetProperty,
            13 => Self::GetIndex,
            14 => Self::SetIndex,
            15 => Self::Add,
            16 => Self::Sub,
            17 => Self::Mul,
            18 => Self::Div,
            19 => Self::Mod,
            20 => Self::Eq,
            21 => Self::Ne,
            22 => Self::Lt,
            23 => Self::Le,
            24 => Self::Gt,
            25 => Self::Ge,
            26 => Self::And,
            27 => Self::Or,
            28 => Self::BitAnd,
            29 => Self::BitOr,
            30 => Self::BitXor,
            31 => Self::Shl,
            32 => Self::Shr,
            33 => Self::Neg,
            34 => Self::Not,
            35 => Self::BitNot,
            36 => Self::NullCoalesce,
            37 => Self::Jump,
            38 => Self::JumpIfFalse,
            39 => Self::JumpIfTrue,
            40 => Self::Loop,
            41 => Self::Call,
            42 => Self::Return,
            43 => Self::NewObject,
            44 => Self::NewArray,
            45 => Self::Print,
            46 => Self::Dup,
            47 => Self::Halt,
            48 => Self::Throw,
            49 => Self::TryBegin,
            50 => Self::TryEnd,
            51 => Self::IsNull,
            52 => Self::ToString,
            53 => Self::IncLocal,
            54 => Self::DecLocal,
            55 => Self::MakeClosure,
            56 => Self::Await,
            57 => Self::GetUpvalue,
            58 => Self::SetUpvalue,
            59 => Self::IsInstance,
            60 => Self::Constant16,
            61 => Self::GetGlobal16,
            62 => Self::SetGlobal16,
            63 => Self::DefineGlobal16,
            64 => Self::StringStartsWith,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Constant => "Constant",
            Self::Null => "Null",
            Self::True => "True",
            Self::False => "False",
            Self::Pop => "Pop",
            Self::GetLocal => "GetLocal",
            Self::SetLocal => "SetLocal",
            Self::GetGlobal => "GetGlobal",
            Self::SetGlobal => "SetGlobal",
            Self::DefineGlobal => "DefineGlobal",
            Self::GetProperty => "GetProperty",
            Self::SetProperty => "SetProperty",
            Self::GetIndex => "GetIndex",
            Self::SetIndex => "SetIndex",
            Self::Add => "Add",
            Self::Sub => "Sub",
            Self::Mul => "Mul",
            Self::Div => "Div",
            Self::Mod => "Mod",
            Self::Eq => "Eq",
            Self::Ne => "Ne",
            Self::Lt => "Lt",
            Self::Le => "Le",
            Self::Gt => "Gt",
            Self::Ge => "Ge",
            Self::And => "And",
            Self::Or => "Or",
            Self::BitAnd => "BitAnd",
            Self::BitOr => "BitOr",
            Self::BitXor => "BitXor",
            Self::Shl => "Shl",
            Self::Shr => "Shr",
            Self::Neg => "Neg",
            Self::Not => "Not",
            Self::BitNot => "BitNot",
            Self::NullCoalesce => "NullCoalesce",
            Self::Jump => "Jump",
            Self::JumpIfFalse => "JumpIfFalse",
            Self::JumpIfTrue => "JumpIfTrue",
            Self::Loop => "Loop",
            Self::Call => "Call",
            Self::Return => "Return",
            Self::NewObject => "NewObject",
            Self::NewArray => "NewArray",
            Self::Print => "Print",
            Self::Dup => "Dup",
            Self::Halt => "Halt",
            Self::Throw => "Throw",
            Self::TryBegin => "TryBegin",
            Self::TryEnd => "TryEnd",
            Self::IsNull => "IsNull",
            Self::ToString => "ToString",
            Self::IncLocal => "IncLocal",
            Self::DecLocal => "DecLocal",
            Self::MakeClosure => "MakeClosure",
            Self::Await => "Await",
            Self::GetUpvalue => "GetUpvalue",
            Self::SetUpvalue => "SetUpvalue",
            Self::IsInstance => "IsInstance",
            Self::Constant16 => "Constant16",
            Self::GetGlobal16 => "GetGlobal16",
            Self::SetGlobal16 => "SetGlobal16",
            Self::DefineGlobal16 => "DefineGlobal16",
            Self::StringStartsWith => "StringStartsWith",
        }
    }
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

    pub fn add_constant(&mut self, value: Value) -> u16 {
        // dedup simple constants
        for (i, c) in self.constants.iter().enumerate() {
            if c == &value {
                return i as u16;
            }
        }
        let idx = self.constants.len();
        if idx > u16::MAX as usize {
            panic!("too many constants (max {})", u16::MAX);
        }
        self.constants.push(value);
        idx as u16
    }

    pub fn emit_constant(&mut self, value: Value, line: usize) {
        let idx = self.add_constant(value);
        if idx <= u8::MAX as u16 {
            self.emit_op(Op::Constant, line);
            self.emit_byte(idx as u8, line);
        } else {
            self.emit_op(Op::Constant16, line);
            self.emit_u16(idx, line);
        }
    }

    /// Emit GetGlobal / GetGlobal16 for a global table index.
    pub fn emit_get_global(&mut self, idx: u16, line: usize) {
        if idx <= u8::MAX as u16 {
            self.emit_op(Op::GetGlobal, line);
            self.emit_byte(idx as u8, line);
        } else {
            self.emit_op(Op::GetGlobal16, line);
            self.emit_u16(idx, line);
        }
    }

    pub fn emit_set_global(&mut self, idx: u16, line: usize) {
        if idx <= u8::MAX as u16 {
            self.emit_op(Op::SetGlobal, line);
            self.emit_byte(idx as u8, line);
        } else {
            self.emit_op(Op::SetGlobal16, line);
            self.emit_u16(idx, line);
        }
    }

    pub fn emit_define_global(&mut self, idx: u16, line: usize) {
        if idx <= u8::MAX as u16 {
            self.emit_op(Op::DefineGlobal, line);
            self.emit_byte(idx as u8, line);
        } else {
            self.emit_op(Op::DefineGlobal16, line);
            self.emit_u16(idx, line);
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ClassKind {
    Class = 0,
    Struct = 1,
    Union = 2,
}

impl ClassKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Struct,
            2 => Self::Union,
            _ => Self::Class,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Union => "union",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub name: String,
    pub kind: ClassKind,
    pub fields: Vec<String>,
    /// Parallel to `fields`: simple type names from the AST (e.g. `"int"`, `"Point"`).
    pub field_types: Vec<String>,
    pub methods: Vec<(String, usize)>, // name -> chunk index
    pub constructor: Option<usize>,
    /// Index of base class in Module.classes, if any.
    pub base: Option<usize>,
    /// Chunk index of destructor (`~new`), if any.
    pub destructor: Option<usize>,
}
