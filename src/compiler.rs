//! AST → bytecode compiler.

use crate::ast::*;
use crate::bytecode::{Chunk, ClassInfo, LocalDebug, Module, Op};
use crate::error::{CompileError, CompileResult};
use crate::ffi::{self, FfiEmbed, FfiModuleInfo};
use crate::span::Span;
use crate::value::{FunctionRef, Value};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

struct Local {
    name: String,
    depth: usize,
    /// `owned` locals call Dispose when leaving scope.
    owned: bool,
    /// Index into `local_ranges` for this live binding.
    range_idx: usize,
}

struct UpvalueDesc {
    name: String,
    /// Local slot or upvalue index in the immediate enclosing function.
    index: u8,
    is_local: bool,
}

struct EnclosingFn {
    locals: Vec<Local>,
    local_ranges: Vec<LocalDebug>,
    upvalues: Vec<UpvalueDesc>,
}

enum NameRes {
    Local(u8),
    Upvalue(u8),
    Global,
}

pub struct Compiler {
    module: Module,
    current: usize, // current chunk index
    locals: Vec<Local>,
    /// Debug live ranges accumulated for the current function.
    local_ranges: Vec<LocalDebug>,
    upvalues: Vec<UpvalueDesc>,
    enclosing: Vec<EnclosingFn>,
    scope_depth: usize,
    max_locals: usize,
    classes: HashMap<String, usize>,
    functions: HashMap<String, usize>,
    /// Bodyless FFI import names (no bytecode chunk used at call time).
    ffi_names: HashSet<String>,
    /// Current default library from nearest [link:]/[DllImport:]
    current_link: Option<String>,
    embed_counter: usize,
    loop_stack: Vec<LoopCtx>,
    errors: Vec<CompileError>,
    /// Source path stamped onto chunks (set by DAP / callers).
    source_path: Option<String>,
    stdlib_enabled: bool,
}

struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

impl Compiler {
    pub fn new() -> Self {
        let mut module = Module {
            chunks: vec![Chunk::new("<script>")],
            main_chunk: 0,
            globals: Vec::new(),
            classes: Vec::new(),
            ffi: FfiModuleInfo::default(),
            stdlib_enabled: true,
        };
        let _ = &mut module;
        Self {
            module,
            current: 0,
            locals: Vec::new(),
            local_ranges: Vec::new(),
            upvalues: Vec::new(),
            enclosing: Vec::new(),
            scope_depth: 0,
            max_locals: 0,
            classes: HashMap::new(),
            functions: HashMap::new(),
            ffi_names: HashSet::new(),
            current_link: None,
            embed_counter: 0,
            loop_stack: Vec::new(),
            errors: Vec::new(),
            source_path: None,
            stdlib_enabled: true,
        }
    }

    pub fn with_stdlib(mut self, stdlib_enabled: bool) -> Self {
        self.stdlib_enabled = stdlib_enabled;
        self.module.stdlib_enabled = stdlib_enabled;
        self
    }

    pub fn with_source(mut self, path: impl Into<String>) -> Self {
        self.source_path = Some(path.into());
        self
    }

    pub fn compile(mut self, program: &Program) -> CompileResult<Module> {
        // First pass: register top-level functions and classes
        for item in &program.items {
            self.declare_item(item)?;
        }
        // Second pass: compile bodies
        for item in &program.items {
            self.compile_item(item)?;
        }

        // Emit call to Main if present
        if self.functions.contains_key("Main") {
            let line = 1;
            self.chunk().emit_op(Op::GetGlobal, line);
            let name_idx = self.ensure_global("Main");
            self.chunk().emit_byte(name_idx, line);
            self.chunk().emit_op(Op::Call, line);
            self.chunk().emit_byte(0, line); // arity
            // If Main is async, await the returned Task before exit
            self.chunk().emit_op(Op::Await, line);
            self.chunk().emit_op(Op::Pop, line);
        }

        self.chunk().emit_op(Op::Halt, 1);

        if let Some(path) = &self.source_path {
            for chunk in &mut self.module.chunks {
                if chunk.source.is_none() {
                    chunk.source = Some(path.clone());
                }
            }
        }

        if let Some(err) = self.errors.first() {
            return Err(CompileError::syntax(err.to_string(), Span::default()));
        }
        Ok(self.module)
    }

    fn chunk(&mut self) -> &mut Chunk {
        &mut self.module.chunks[self.current]
    }

    fn ensure_global(&mut self, name: &str) -> u8 {
        if let Some(i) = self.module.globals.iter().position(|g| g == name) {
            return i as u8;
        }
        let i = self.module.globals.len();
        self.module.globals.push(name.to_string());
        i as u8
    }

    fn declare_item(&mut self, item: &Item) -> CompileResult<()> {
        match item {
            Item::Attribute(_, inner) => self.declare_item(inner),
            Item::Namespace(ns) => {
                for i in &ns.items {
                    self.declare_item(i)?;
                }
                Ok(())
            }
            Item::Function(f) => {
                if ffi::is_ffi_import(f) {
                    self.ffi_names.insert(f.name.clone());
                    self.ensure_global(&f.name);
                    return Ok(());
                }
                let idx = self.module.chunks.len();
                let mut chunk = Chunk::new(&f.name);
                chunk.arity = f.params.len();
                self.module.chunks.push(chunk);
                self.functions.insert(f.name.clone(), idx);
                let _ = self.ensure_global(&f.name);
                // Extension method: also register as Type.Name for instance dispatch
                if f.is_extension {
                    if let Some(p) = f.params.first() {
                        let key = format!("{}.{}", p.ty.name, f.name);
                        self.functions.insert(key.clone(), idx);
                        let _ = self.ensure_global(&key);
                    }
                }
                Ok(())
            }
            Item::Class(c) => {
                let class_idx = self.module.classes.len();
                let mut info = ClassInfo {
                    name: c.name.clone(),
                    fields: Vec::new(),
                    methods: Vec::new(),
                    constructor: None,
                    base: c.bases.first().and_then(|b| self.classes.get(&b.name).copied()),
                    destructor: None,
                };
                for m in &c.members {
                    match m {
                        Member::Field(f) => {
                            if !f.is_static {
                                info.fields.push(f.name.clone())
                            }
                        }
                        Member::Property(p) => {
                            if !p.is_static {
                                info.fields.push(p.name.clone())
                            }
                        }
                        Member::Method(f) => {
                            let idx = self.module.chunks.len();
                            let mut chunk = Chunk::new(format!("{}.{}", c.name, f.name));
                            chunk.arity = f.params.len() + usize::from(!f.is_static);
                            self.module.chunks.push(chunk);
                            info.methods.push((f.name.clone(), idx));
                            self.functions
                                .insert(format!("{}.{}", c.name, f.name), idx);
                        }
                        Member::Constructor(ctor) => {
                            let idx = self.module.chunks.len();
                            let mut chunk = Chunk::new(format!("{}.new", c.name));
                            chunk.arity = ctor.params.len() + 1;
                            self.module.chunks.push(chunk);
                            info.constructor = Some(idx);
                            self.functions.insert(format!("{}.new", c.name), idx);
                        }
                        Member::Destructor(_) => {
                            let idx = self.module.chunks.len();
                            let mut chunk = Chunk::new(format!("{}.~new", c.name));
                            chunk.arity = 1; // this
                            self.module.chunks.push(chunk);
                            info.destructor = Some(idx);
                            self.functions.insert(format!("{}.~new", c.name), idx);
                        }
                        Member::Operator(op) => {
                            let idx = self.module.chunks.len();
                            let mut chunk = Chunk::new(format!("{}.operator{}", c.name, op.op));
                            chunk.arity = op.params.len();
                            self.module.chunks.push(chunk);
                            info.methods
                                .push((format!("operator{}", op.op), idx));
                            self.functions
                                .insert(format!("{}.operator{}", c.name, op.op), idx);
                        }
                        Member::Indexer(idxer) => {
                            let gidx = self.module.chunks.len();
                            let mut chunk = Chunk::new(format!("{}.get_Item", c.name));
                            chunk.arity = idxer.params.len() + 1;
                            self.module.chunks.push(chunk);
                            info.methods.push(("get_Item".into(), gidx));
                            self.functions
                                .insert(format!("{}.get_Item", c.name), gidx);
                            if idxer.setter.is_some() {
                                let sidx = self.module.chunks.len();
                                let mut chunk = Chunk::new(format!("{}.set_Item", c.name));
                                chunk.arity = idxer.params.len() + 2; // this, indices..., value
                                self.module.chunks.push(chunk);
                                info.methods.push(("set_Item".into(), sidx));
                                self.functions
                                    .insert(format!("{}.set_Item", c.name), sidx);
                            }
                        }
                    }
                }
                self.classes.insert(c.name.clone(), class_idx);
                self.module.classes.push(info);
                // Also register class name as global constructor helper
                self.ensure_global(&c.name);
                Ok(())
            }
            Item::Struct(s) => {
                // Treat structs like classes for VM
                let class_idx = self.module.classes.len();
                let mut info = ClassInfo {
                    name: s.name.clone(),
                    fields: Vec::new(),
                    methods: Vec::new(),
                    constructor: None,
                    base: None,
                    destructor: None,
                };
                for m in &s.members {
                    match m {
                        Member::Field(f) => {
                            if !f.is_static {
                                info.fields.push(f.name.clone())
                            }
                        }
                        Member::Method(f) => {
                            let idx = self.module.chunks.len();
                            let mut chunk = Chunk::new(format!("{}.{}", s.name, f.name));
                            chunk.arity = f.params.len() + usize::from(!f.is_static);
                            self.module.chunks.push(chunk);
                            info.methods.push((f.name.clone(), idx));
                            self.functions
                                .insert(format!("{}.{}", s.name, f.name), idx);
                        }
                        Member::Constructor(ctor) => {
                            let idx = self.module.chunks.len();
                            let mut chunk = Chunk::new(format!("{}.new", s.name));
                            chunk.arity = ctor.params.len() + 1;
                            self.module.chunks.push(chunk);
                            info.constructor = Some(idx);
                            self.functions.insert(format!("{}.new", s.name), idx);
                        }
                        _ => {}
                    }
                }
                self.classes.insert(s.name.clone(), class_idx);
                self.module.classes.push(info);
                self.ensure_global(&s.name);
                Ok(())
            }
            Item::Interface(_) | Item::Import(_) | Item::Module(_) | Item::Const(_) => Ok(()),
        }
    }

    fn compile_item(&mut self, item: &Item) -> CompileResult<()> {
        let (attrs, core) = peel_attributes(item);
        self.apply_ffi_attrs(&attrs)?;
        match core {
            Item::Namespace(ns) => {
                for i in &ns.items {
                    self.compile_item(i)?;
                }
                Ok(())
            }
            Item::Function(f) => self.compile_function(f, &attrs),
            Item::Class(c) => self.compile_class(c),
            Item::Struct(s) => self.compile_struct(s),
            Item::Const(c) => {
                let line = c.span.line;
                self.compile_expr(&c.value)?;
                let g = self.ensure_global(&c.name);
                self.chunk().emit_op(Op::DefineGlobal, line);
                self.chunk().emit_byte(g, line);
                Ok(())
            }
            Item::Attribute(_, _) => unreachable!("peeled"),
            Item::Import(_) | Item::Module(_) | Item::Interface(_) => Ok(()),
        }
    }

    fn apply_ffi_attrs(&mut self, attrs: &[&Attribute]) -> CompileResult<()> {
        for a in attrs {
            let key = a.name.to_ascii_lowercase();
            match key.as_str() {
                "include" | "bind" | "cheader" => {
                    if let Some(s) = ffi::attr_string(a) {
                        if !self.module.ffi.includes.contains(&s) {
                            self.module.ffi.includes.push(s);
                        }
                        // Prototypes are expanded into AST by ffi_bind (VM, no gcc).
                        // Path is still recorded for native C codegen `#include`.
                    }
                }
                "link" | "dllimport" | "lib" => {
                    if let Some(s) = ffi::attr_string(a) {
                        self.current_link = Some(s.clone());
                        if !self.module.ffi.links.contains(&s) {
                            self.module.ffi.links.push(s);
                        }
                    }
                }
                "c" | "cembed" | "embed" => {
                    if let Some(source) = ffi::attr_string(a) {
                        self.embed_counter += 1;
                        let lib_name = format!("raytask_embed_{}", self.embed_counter);
                        self.module.ffi.embeds.push(FfiEmbed {
                            source,
                            lib_name: lib_name.clone(),
                        });
                        self.current_link = Some(lib_name.clone());
                        if !self.module.ffi.links.contains(&lib_name) {
                            self.module.ffi.links.push(lib_name);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn compile_function(&mut self, f: &FunctionDecl, outer_attrs: &[&Attribute]) -> CompileResult<()> {
        let mut all_attrs: Vec<Attribute> = outer_attrs.iter().map(|a| (*a).clone()).collect();
        all_attrs.extend(f.attributes.iter().cloned());

        if ffi::is_ffi_import(f) || self.ffi_names.contains(&f.name) {
            let ffi_fn = ffi::ffi_from_function(f, self.current_link.as_deref(), &all_attrs)
                .map_err(|m| CompileError::syntax(m, f.span))?;
            let line = f.span.line;
            let prev = self.current;
            self.current = 0;
            self.chunk().emit_constant(Value::Ffi(ffi_fn), line);
            let g = self.ensure_global(&f.name);
            self.chunk().emit_op(Op::DefineGlobal, line);
            self.chunk().emit_byte(g, line);
            self.current = prev;
            return Ok(());
        }

        let Some(&idx) = self.functions.get(&f.name) else {
            return Ok(());
        };
        self.module.chunks[idx].is_async = f.is_async;
        let prev = self.current;
        self.current = idx;
        self.reset_function_locals();
        self.begin_scope();
        for p in &f.params {
            self.add_local(&p.name);
        }
        match &f.body {
            Some(FunctionBody::Block(b)) => self.compile_block(b)?,
            Some(FunctionBody::Expr(e)) => {
                self.compile_expr(e)?;
                self.chunk().emit_op(Op::Return, f.span.line);
            }
            None => {}
        }
        // Implicit return null
        self.chunk().emit_op(Op::Null, f.span.line);
        self.chunk().emit_op(Op::Return, f.span.line);
        self.end_scope();
        self.finish_locals(idx);
        self.current = prev;

        // Define global function value in main chunk
        let line = f.span.line;
        let prev = self.current;
        self.current = 0;
        let export_name = all_attrs
            .iter()
            .find(|a| match a.name.to_ascii_lowercase().as_str() {
                "export" | "entry" | "symbol" | "name" => true,
                _ => false,
            })
            .and_then(ffi::attr_string);
        let _ = export_name; // used by C codegen via attributes on AST
        let func = Value::Function(FunctionRef {
            name: f.name.clone(),
            chunk_index: idx,
            arity: f.params.len(),
            defaults: vec![],
            is_async: f.is_async,
            upvalues: vec![],
        });
        self.chunk().emit_constant(func, line);
        let g = self.ensure_global(&f.name);
        self.chunk().emit_op(Op::DefineGlobal, line);
        self.chunk().emit_byte(g, line);
        if f.is_extension {
            if let Some(p) = f.params.first() {
                let key = format!("{}.{}", p.ty.name, f.name);
                self.chunk().emit_constant(
                    Value::Function(FunctionRef {
                        name: key.clone(),
                        chunk_index: idx,
                        arity: f.params.len(),
                        defaults: vec![],
                        is_async: f.is_async,
                        upvalues: vec![],
                    }),
                    line,
                );
                let g2 = self.ensure_global(&key);
                self.chunk().emit_op(Op::DefineGlobal, line);
                self.chunk().emit_byte(g2, line);
            }
        }
        self.current = prev;
        Ok(())
    }

    fn compile_class(&mut self, c: &ClassDecl) -> CompileResult<()> {
        for m in &c.members {
            match m {
                Member::Method(f) => {
                    let key = format!("{}.{}", c.name, f.name);
                    if let Some(&idx) = self.functions.get(&key) {
                        self.compile_method_body(idx, f, !f.is_static)?;
                    }
                }
                Member::Constructor(ctor) => {
                    let key = format!("{}.new", c.name);
                    if let Some(&idx) = self.functions.get(&key) {
                        self.compile_constructor(idx, ctor, &c.name, &c.bases)?;
                    }
                }
                Member::Destructor(d) => {
                    let key = format!("{}.~new", c.name);
                    if let Some(&idx) = self.functions.get(&key) {
                        let fake = FunctionDecl {
                            access: Access::Default,
                            is_async: false,
                            is_unsafe: false,
                            is_static: false,
                            is_virtual: false,
                            is_override: false,
                            is_abstract: false,
                            is_extension: false,
                            return_type: TypeRef::void(d.span),
                            name: "~new".into(),
                            type_params: vec![],
                            params: vec![],
                            constraints: vec![],
                            body: Some(FunctionBody::Block(d.body.clone())),
                            attributes: vec![],
                            span: d.span,
                        };
                        self.compile_method_body(idx, &fake, false)?;
                    }
                }
                Member::Property(p) => {
                    if p.auto {
                        continue;
                    }
                    if let Some(getter) = &p.getter {
                        let key = format!("{}.get_{}", c.name, p.name);
                        let idx = self.module.chunks.len();
                        let mut chunk = Chunk::new(&key);
                        chunk.arity = usize::from(!p.is_static);
                        self.module.chunks.push(chunk);
                        self.functions.insert(key.clone(), idx);
                        if let Some(ci) = self.classes.get(&c.name).copied() {
                            self.module.classes[ci]
                                .methods
                                .push((format!("get_{}", p.name), idx));
                        }
                        let fake = FunctionDecl {
                            access: Access::Default,
                            is_async: false,
                            is_unsafe: false,
                            is_static: p.is_static,
                            is_virtual: false,
                            is_override: false,
                            is_abstract: false,
                            is_extension: false,
                            return_type: p.ty.clone(),
                            name: format!("get_{}", p.name),
                            type_params: vec![],
                            params: vec![],
                            constraints: vec![],
                            body: Some(FunctionBody::Block(getter.clone())),
                            attributes: vec![],
                            span: p.span,
                        };
                        self.compile_method_body(idx, &fake, !p.is_static)?;
                    }
                    if let Some(setter) = &p.setter {
                        let key = format!("{}.set_{}", c.name, p.name);
                        let idx = self.module.chunks.len();
                        let mut chunk = Chunk::new(&key);
                        chunk.arity = 1 + usize::from(!p.is_static);
                        self.module.chunks.push(chunk);
                        self.functions.insert(key.clone(), idx);
                        if let Some(ci) = self.classes.get(&c.name).copied() {
                            self.module.classes[ci]
                                .methods
                                .push((format!("set_{}", p.name), idx));
                        }
                        let fake = FunctionDecl {
                            access: Access::Default,
                            is_async: false,
                            is_unsafe: false,
                            is_static: p.is_static,
                            is_virtual: false,
                            is_override: false,
                            is_abstract: false,
                            is_extension: false,
                            return_type: TypeRef::void(p.span),
                            name: format!("set_{}", p.name),
                            type_params: vec![],
                            params: vec![Param {
                                is_params: false,
                                is_this: false,
                                name: "value".into(),
                                ty: p.ty.clone(),
                                default: None,
                                span: p.span,
                            }],
                            constraints: vec![],
                            body: Some(FunctionBody::Block(setter.clone())),
                            attributes: vec![],
                            span: p.span,
                        };
                        self.compile_method_body(idx, &fake, !p.is_static)?;
                    }
                }
                Member::Operator(op) => {
                    let key = format!("{}.operator{}", c.name, op.op);
                    if let Some(&idx) = self.functions.get(&key) {
                        let fake = FunctionDecl {
                            access: Access::Default,
                            is_async: false,
                            is_unsafe: false,
                            is_static: false,
                            is_virtual: false,
                            is_override: false,
                            is_abstract: false,
                            is_extension: false,
                            return_type: op.return_type.clone(),
                            name: format!("operator{}", op.op),
                            type_params: vec![],
                            params: op.params.clone(),
                            constraints: vec![],
                            body: Some(FunctionBody::Block(op.body.clone())),
                            attributes: vec![],
                            span: op.span,
                        };
                        self.compile_method_body(idx, &fake, false)?;
                        let line = op.span.line;
                        let prev = self.current;
                        self.current = 0;
                        let func = Value::Function(FunctionRef {
                            name: key.clone(),
                            chunk_index: idx,
                            arity: op.params.len(),
                            defaults: vec![],
                            is_async: false,
                            upvalues: vec![],
                        });
                        self.chunk().emit_constant(func, line);
                        let g = self.ensure_global(&key);
                        self.chunk().emit_op(Op::DefineGlobal, line);
                        self.chunk().emit_byte(g, line);
                        self.current = prev;
                    }
                }
                Member::Indexer(idxer) => {
                    if let Some(getter) = &idxer.getter {
                        let key = format!("{}.get_Item", c.name);
                        if let Some(&idx) = self.functions.get(&key) {
                            let fake = FunctionDecl {
                                access: Access::Default,
                                is_async: false,
                                is_unsafe: false,
                                is_static: false,
                                is_virtual: false,
                                is_override: false,
                                is_abstract: false,
                                is_extension: false,
                                return_type: idxer.ty.clone(),
                                name: "get_Item".into(),
                                type_params: vec![],
                                params: idxer.params.clone(),
                                constraints: vec![],
                                body: Some(FunctionBody::Block(getter.clone())),
                                attributes: vec![],
                                span: idxer.span,
                            };
                            self.compile_method_body(idx, &fake, true)?;
                        }
                    }
                    if let Some(setter) = &idxer.setter {
                        let key = format!("{}.set_Item", c.name);
                        if let Some(&idx) = self.functions.get(&key) {
                            let mut params = idxer.params.clone();
                            params.push(Param {
                                is_params: false,
                                is_this: false,
                                name: "value".into(),
                                ty: idxer.ty.clone(),
                                default: None,
                                span: idxer.span,
                            });
                            let fake = FunctionDecl {
                                access: Access::Default,
                                is_async: false,
                                is_unsafe: false,
                                is_static: false,
                                is_virtual: false,
                                is_override: false,
                                is_abstract: false,
                                is_extension: false,
                                return_type: TypeRef::void(idxer.span),
                                name: "set_Item".into(),
                                type_params: vec![],
                                params,
                                constraints: vec![],
                                body: Some(FunctionBody::Block(setter.clone())),
                                attributes: vec![],
                                span: idxer.span,
                            };
                            self.compile_method_body(idx, &fake, true)?;
                        }
                    }
                }
                _ => {}
            }
        }
        self.emit_static_members(&c.name, &c.members)?;
        // Register class factory as global
        let line = c.span.line;
        let prev = self.current;
        self.current = 0;
        if let Some(&ci) = self.classes.get(&c.name) {
            let _ = ci;
            self.chunk()
                .emit_constant(Value::TypeModule(c.name.clone().into()), line);
            let g = self.ensure_global(&c.name);
            self.chunk().emit_op(Op::DefineGlobal, line);
            self.chunk().emit_byte(g, line);
        }
        self.current = prev;
        Ok(())
    }

    fn compile_struct(&mut self, s: &StructDecl) -> CompileResult<()> {
        // Mirror class compilation
        for m in &s.members {
            match m {
                Member::Method(f) => {
                    let key = format!("{}.{}", s.name, f.name);
                    if let Some(&idx) = self.functions.get(&key) {
                        self.compile_method_body(idx, f, !f.is_static)?;
                    }
                }
                Member::Constructor(ctor) => {
                    let key = format!("{}.new", s.name);
                    if let Some(&idx) = self.functions.get(&key) {
                        self.compile_constructor(idx, ctor, &s.name, &[])?;
                    }
                }
                _ => {}
            }
        }
        self.emit_static_members(&s.name, &s.members)?;
        let line = s.span.line;
        let prev = self.current;
        self.current = 0;
        if let Some(&ci) = self.classes.get(&s.name) {
            let _ = ci;
            self.chunk()
                .emit_constant(Value::TypeModule(s.name.clone().into()), line);
            let g = self.ensure_global(&s.name);
            self.chunk().emit_op(Op::DefineGlobal, line);
            self.chunk().emit_byte(g, line);
        }
        self.current = prev;
        Ok(())
    }

    fn compile_method_body(
        &mut self,
        idx: usize,
        f: &FunctionDecl,
        has_this: bool,
    ) -> CompileResult<()> {
        self.module.chunks[idx].is_async = f.is_async;
        let prev = self.current;
        self.current = idx;
        self.reset_function_locals();
        self.begin_scope();
        if has_this {
            self.add_local("this");
        }
        for p in &f.params {
            self.add_local(&p.name);
        }
        match &f.body {
            Some(FunctionBody::Block(b)) => self.compile_block(b)?,
            Some(FunctionBody::Expr(e)) => {
                self.compile_expr(e)?;
                self.chunk().emit_op(Op::Return, f.span.line);
            }
            None => {}
        }
        self.chunk().emit_op(Op::Null, f.span.line);
        self.chunk().emit_op(Op::Return, f.span.line);
        self.end_scope();
        self.finish_locals(idx);
        self.current = prev;
        Ok(())
    }

    fn emit_static_members(&mut self, type_name: &str, members: &[Member]) -> CompileResult<()> {
        let prev = self.current;
        self.current = 0;
        for m in members {
            match m {
                Member::Field(f) if f.is_static => {
                    let key = format!("{}.{}", type_name, f.name);
                    if let Some(init) = &f.init {
                        self.compile_expr(init)?;
                    } else {
                        self.chunk().emit_op(Op::Null, f.span.line);
                    }
                    let g = self.ensure_global(&key);
                    self.chunk().emit_op(Op::DefineGlobal, f.span.line);
                    self.chunk().emit_byte(g, f.span.line);
                }
                Member::Property(p) if p.is_static && p.auto => {
                    let key = format!("{}.{}", type_name, p.name);
                    self.chunk().emit_op(Op::Null, p.span.line);
                    let g = self.ensure_global(&key);
                    self.chunk().emit_op(Op::DefineGlobal, p.span.line);
                    self.chunk().emit_byte(g, p.span.line);
                }
                _ => {}
            }
        }
        self.current = prev;
        Ok(())
    }

    fn compile_constructor(
        &mut self,
        idx: usize,
        ctor: &ConstructorDecl,
        class_name: &str,
        bases: &[TypeRef],
    ) -> CompileResult<()> {
        let prev = self.current;
        self.current = idx;
        self.reset_function_locals();
        self.begin_scope();
        self.add_local("this");
        for p in &ctor.params {
            self.add_local(&p.name);
        }
        // base(...) — call base constructor on this
        if !ctor.base_args.is_empty() {
            if let Some(base) = bases.first() {
                let base_ctor = format!("{}.new", base.name);
                if let Some(&bidx) = self.functions.get(&base_ctor) {
                    let line = ctor.span.line;
                    self.chunk().emit_constant(
                        Value::Function(FunctionRef {
                            name: base_ctor,
                            chunk_index: bidx,
                            arity: ctor.base_args.len() + 1,
                            defaults: vec![],
                            is_async: false,
                            upvalues: vec![],
                        }),
                        line,
                    );
                    self.chunk().emit_op(Op::GetLocal, line);
                    self.chunk().emit_byte(0, line); // this
                    for a in &ctor.base_args {
                        self.compile_expr(a)?;
                    }
                    self.chunk().emit_op(Op::Call, line);
                    self.chunk()
                        .emit_byte((ctor.base_args.len() + 1) as u8, line);
                    self.chunk().emit_op(Op::Pop, line);
                }
            }
        } else if bases.first().is_some() {
            // Implicit base() if base has zero-arg ctor
            let _ = class_name;
        }
        self.compile_block(&ctor.body)?;
        // return this
        self.chunk().emit_op(Op::GetLocal, ctor.span.line);
        self.chunk().emit_byte(0, ctor.span.line);
        self.chunk().emit_op(Op::Return, ctor.span.line);
        self.end_scope();
        self.finish_locals(idx);
        self.current = prev;
        Ok(())
    }

    fn begin_scope(&mut self) {
        self.scope_depth += 1;
    }

    fn end_scope(&mut self) {
        self.scope_depth -= 1;
        while self
            .locals
            .last()
            .map(|l| l.depth > self.scope_depth)
            .unwrap_or(false)
        {
            let slot = (self.locals.len() - 1) as u8;
            let local = self.locals.pop().expect("local");
            let end_ip = self.module.chunks[self.current].code.len();
            if let Some(range) = self.local_ranges.get_mut(local.range_idx) {
                range.end_ip = end_ip;
            }
            if local.owned {
                let line = 1;
                // obj.Dispose() — same stack shape as method call
                self.chunk().emit_op(Op::GetLocal, line);
                self.chunk().emit_byte(slot, line);
                self.chunk().emit_op(Op::Dup, line);
                self.chunk()
                    .emit_constant(Value::String("Dispose".into()), line);
                self.chunk().emit_op(Op::GetProperty, line);
                self.chunk().emit_op(Op::Call, line);
                self.chunk().emit_byte(1, line); // this
                self.chunk().emit_op(Op::Pop, line);
            }
        }
    }

    fn add_local(&mut self, name: &str) {
        self.add_local_ex(name, false);
    }

    fn add_local_ex(&mut self, name: &str, owned: bool) {
        let slot = self.locals.len() as u8;
        let start_ip = self.module.chunks[self.current].code.len();
        let range_idx = self.local_ranges.len();
        self.local_ranges.push(LocalDebug {
            name: name.to_string(),
            slot,
            start_ip,
            end_ip: usize::MAX,
        });
        self.locals.push(Local {
            name: name.to_string(),
            depth: self.scope_depth,
            owned,
            range_idx,
        });
        self.max_locals = self.max_locals.max(self.locals.len());
    }

    fn finish_locals(&mut self, chunk_idx: usize) {
        let end_ip = self.module.chunks[chunk_idx].code.len();
        for range in &mut self.local_ranges {
            if range.end_ip == usize::MAX {
                range.end_ip = end_ip;
            }
        }
        self.module.chunks[chunk_idx].local_count = self.max_locals.max(self.locals.len());
        self.module.chunks[chunk_idx].local_debug = std::mem::take(&mut self.local_ranges);
        if let Some(path) = &self.source_path {
            self.module.chunks[chunk_idx].source = Some(path.clone());
        }
    }

    fn reset_function_locals(&mut self) {
        self.locals.clear();
        self.local_ranges.clear();
        self.scope_depth = 0;
        self.max_locals = 0;
    }

    fn resolve_local(&self, name: &str) -> Option<u8> {
        for (i, local) in self.locals.iter().enumerate().rev() {
            if local.name == name {
                return Some(i as u8);
            }
        }
        None
    }

    fn resolve_name(&mut self, name: &str) -> NameRes {
        if let Some(slot) = self.resolve_local(name) {
            return NameRes::Local(slot);
        }
        if let Some(uv) = self.add_upvalue(name) {
            return NameRes::Upvalue(uv);
        }
        NameRes::Global
    }

    fn add_upvalue(&mut self, name: &str) -> Option<u8> {
        for (i, u) in self.upvalues.iter().enumerate() {
            if u.name == name {
                return Some(i as u8);
            }
        }
        let (is_local, index) = Self::capture_chain(&mut self.enclosing, name)?;
        if self.upvalues.len() >= 255 {
            panic!("too many upvalues");
        }
        let idx = self.upvalues.len() as u8;
        self.upvalues.push(UpvalueDesc {
            name: name.to_string(),
            index,
            is_local,
        });
        Some(idx)
    }

    /// Resolve `name` through enclosing frames, adding upvalues as needed.
    /// Returns (is_local, index) relative to the *immediate* enclosing function.
    fn capture_chain(enclosing: &mut [EnclosingFn], name: &str) -> Option<(bool, u8)> {
        if enclosing.is_empty() {
            return None;
        }
        let last = enclosing.len() - 1;
        for (i, local) in enclosing[last].locals.iter().enumerate().rev() {
            if local.name == name {
                return Some((true, i as u8));
            }
        }
        for (i, u) in enclosing[last].upvalues.iter().enumerate() {
            if u.name == name {
                return Some((false, i as u8));
            }
        }
        let (is_local, index) = {
            let (head, _) = enclosing.split_at_mut(last);
            Self::capture_chain(head, name)?
        };
        if enclosing[last].upvalues.len() >= 255 {
            panic!("too many upvalues");
        }
        let uv_idx = enclosing[last].upvalues.len() as u8;
        enclosing[last].upvalues.push(UpvalueDesc {
            name: name.to_string(),
            index,
            is_local,
        });
        Some((false, uv_idx))
    }

    fn emit_get_name(&mut self, name: &str, line: usize) {
        match self.resolve_name(name) {
            NameRes::Local(slot) => {
                self.chunk().emit_op(Op::GetLocal, line);
                self.chunk().emit_byte(slot, line);
            }
            NameRes::Upvalue(idx) => {
                self.chunk().emit_op(Op::GetUpvalue, line);
                self.chunk().emit_byte(idx, line);
            }
            NameRes::Global => {
                let g = self.ensure_global(name);
                self.chunk().emit_op(Op::GetGlobal, line);
                self.chunk().emit_byte(g, line);
            }
        }
    }

    fn emit_set_name(&mut self, name: &str, line: usize) {
        match self.resolve_name(name) {
            NameRes::Local(slot) => {
                self.chunk().emit_op(Op::SetLocal, line);
                self.chunk().emit_byte(slot, line);
            }
            NameRes::Upvalue(idx) => {
                self.chunk().emit_op(Op::SetUpvalue, line);
                self.chunk().emit_byte(idx, line);
            }
            NameRes::Global => {
                let g = self.ensure_global(name);
                self.chunk().emit_op(Op::SetGlobal, line);
                self.chunk().emit_byte(g, line);
            }
        }
    }

    fn compile_switch_body(
        &mut self,
        body: &[Stmt],
        end_jumps: &mut Vec<usize>,
        line: usize,
    ) -> CompileResult<()> {
        for s in body {
            match s {
                Stmt::Break(_) => {
                    let j = self.chunk().emit_jump(Op::Jump, line);
                    end_jumps.push(j);
                }
                other => self.compile_stmt(other)?,
            }
        }
        Ok(())
    }

    fn compile_block(&mut self, block: &Block) -> CompileResult<()> {
        self.begin_scope();
        for stmt in &block.stmts {
            self.compile_stmt(stmt)?;
        }
        self.end_scope();
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> CompileResult<()> {
        match stmt {
            Stmt::Expr(e) => {
                self.compile_expr(e)?;
                self.chunk().emit_op(Op::Pop, e.span().line);
            }
            Stmt::Decl(d) => {
                if let Some(init) = &d.init {
                    self.compile_expr(init)?;
                } else {
                    self.chunk().emit_op(Op::Null, d.span.line);
                }
                if self.scope_depth > 0 {
                    let owned = d.kind == VarKind::Owned;
                    self.add_local_ex(&d.name, owned);
                    let slot = (self.locals.len() - 1) as u8;
                    self.chunk().emit_op(Op::SetLocal, d.span.line);
                    self.chunk().emit_byte(slot, d.span.line);
                    self.chunk().emit_op(Op::Pop, d.span.line);
                } else {
                    let g = self.ensure_global(&d.name);
                    self.chunk().emit_op(Op::DefineGlobal, d.span.line);
                    self.chunk().emit_byte(g, d.span.line);
                }
            }
            Stmt::Const(c) => {
                self.compile_expr(&c.value)?;
                if self.scope_depth > 0 {
                    self.add_local(&c.name);
                    let slot = (self.locals.len() - 1) as u8;
                    self.chunk().emit_op(Op::SetLocal, c.span.line);
                    self.chunk().emit_byte(slot, c.span.line);
                    self.chunk().emit_op(Op::Pop, c.span.line);
                } else {
                    let g = self.ensure_global(&c.name);
                    self.chunk().emit_op(Op::DefineGlobal, c.span.line);
                    self.chunk().emit_byte(g, c.span.line);
                }
            }
            Stmt::Return(expr, span) => {
                if let Some(e) = expr {
                    self.compile_expr(e)?;
                } else {
                    self.chunk().emit_op(Op::Null, span.line);
                }
                self.chunk().emit_op(Op::Return, span.line);
            }
            Stmt::If {
                cond,
                then_block,
                else_branch,
                span,
            } => {
                self.compile_expr(cond)?;
                let then_jump = self.chunk().emit_jump(Op::JumpIfFalse, span.line);
                self.chunk().emit_op(Op::Pop, span.line);
                self.compile_block(then_block)?;
                let else_jump = self.chunk().emit_jump(Op::Jump, span.line);
                self.chunk().patch_jump(then_jump);
                self.chunk().emit_op(Op::Pop, span.line);
                if let Some(else_b) = else_branch {
                    match else_b {
                        ElseBranch::Block(b) => self.compile_block(b)?,
                        ElseBranch::If(s) => self.compile_stmt(s)?,
                    }
                }
                self.chunk().patch_jump(else_jump);
            }
            Stmt::While { cond, body, span } => {
                let loop_start = self.chunk().code.len();
                self.loop_stack.push(LoopCtx {
                    breaks: vec![],
                    continues: vec![],
                });
                self.compile_expr(cond)?;
                let exit = self.chunk().emit_jump(Op::JumpIfFalse, span.line);
                self.chunk().emit_op(Op::Pop, span.line);
                self.compile_block(body)?;
                // patch continues
                if let Some(ctx) = self.loop_stack.last_mut() {
                    let conts = std::mem::take(&mut ctx.continues);
                    for c in conts {
                        self.chunk().patch_jump(c);
                    }
                }
                self.chunk().emit_loop(loop_start, span.line);
                self.chunk().patch_jump(exit);
                self.chunk().emit_op(Op::Pop, span.line);
                if let Some(ctx) = self.loop_stack.pop() {
                    for b in ctx.breaks {
                        self.chunk().patch_jump(b);
                    }
                }
            }
            Stmt::DoWhile { body, cond, span } => {
                let loop_start = self.chunk().code.len();
                self.loop_stack.push(LoopCtx {
                    breaks: vec![],
                    continues: vec![],
                });
                self.compile_block(body)?;
                if let Some(ctx) = self.loop_stack.last_mut() {
                    let conts = std::mem::take(&mut ctx.continues);
                    for c in conts {
                        self.chunk().patch_jump(c);
                    }
                }
                self.compile_expr(cond)?;
                let exit = self.chunk().emit_jump(Op::JumpIfFalse, span.line);
                self.chunk().emit_op(Op::Pop, span.line);
                self.chunk().emit_loop(loop_start, span.line);
                self.chunk().patch_jump(exit);
                self.chunk().emit_op(Op::Pop, span.line);
                if let Some(ctx) = self.loop_stack.pop() {
                    for b in ctx.breaks {
                        self.chunk().patch_jump(b);
                    }
                }
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
                span,
            } => {
                self.begin_scope();
                if let Some(i) = init {
                    self.compile_stmt(i)?;
                }
                let loop_start = self.chunk().code.len();
                self.loop_stack.push(LoopCtx {
                    breaks: vec![],
                    continues: vec![],
                });
                let exit = if let Some(c) = cond {
                    self.compile_expr(c)?;
                    let exit = self.chunk().emit_jump(Op::JumpIfFalse, span.line);
                    self.chunk().emit_op(Op::Pop, span.line);
                    Some(exit)
                } else {
                    None
                };
                self.compile_block(body)?;
                if let Some(ctx) = self.loop_stack.last_mut() {
                    let conts = std::mem::take(&mut ctx.continues);
                    for c in conts {
                        self.chunk().patch_jump(c);
                    }
                }
                if let Some(s) = step {
                    self.compile_expr(s)?;
                    self.chunk().emit_op(Op::Pop, span.line);
                }
                self.chunk().emit_loop(loop_start, span.line);
                if let Some(exit) = exit {
                    self.chunk().patch_jump(exit);
                    self.chunk().emit_op(Op::Pop, span.line);
                }
                if let Some(ctx) = self.loop_stack.pop() {
                    for b in ctx.breaks {
                        self.chunk().patch_jump(b);
                    }
                }
                self.end_scope();
            }
            Stmt::Foreach {
                var_name,
                index_name,
                iter,
                body,
                span,
            } => {
                // Desugar to for loop over array
                self.begin_scope();
                self.compile_expr(iter)?;
                self.add_local("__iter");
                let iter_slot = (self.locals.len() - 1) as u8;
                self.chunk().emit_op(Op::SetLocal, span.line);
                self.chunk().emit_byte(iter_slot, span.line);
                self.chunk().emit_op(Op::Pop, span.line);

                self.chunk().emit_constant(Value::Int(0), span.line);
                self.add_local("__i");
                let i_slot = (self.locals.len() - 1) as u8;
                self.chunk().emit_op(Op::SetLocal, span.line);
                self.chunk().emit_byte(i_slot, span.line);
                self.chunk().emit_op(Op::Pop, span.line);

                let loop_start = self.chunk().code.len();
                self.loop_stack.push(LoopCtx {
                    breaks: vec![],
                    continues: vec![],
                });

                // condition: i < iter.Count — use length via GetProperty Length or array len
                self.chunk().emit_op(Op::GetLocal, span.line);
                self.chunk().emit_byte(i_slot, span.line);
                self.chunk().emit_op(Op::GetLocal, span.line);
                self.chunk().emit_byte(iter_slot, span.line);
                self.chunk()
                    .emit_constant(Value::String("Length".into()), span.line);
                self.chunk().emit_op(Op::GetProperty, span.line);
                self.chunk().emit_op(Op::Lt, span.line);
                let exit = self.chunk().emit_jump(Op::JumpIfFalse, span.line);
                self.chunk().emit_op(Op::Pop, span.line);

                // var item = iter[i]
                self.chunk().emit_op(Op::GetLocal, span.line);
                self.chunk().emit_byte(iter_slot, span.line);
                self.chunk().emit_op(Op::GetLocal, span.line);
                self.chunk().emit_byte(i_slot, span.line);
                self.chunk().emit_op(Op::GetIndex, span.line);
                self.add_local(var_name);
                let v_slot = (self.locals.len() - 1) as u8;
                self.chunk().emit_op(Op::SetLocal, span.line);
                self.chunk().emit_byte(v_slot, span.line);
                self.chunk().emit_op(Op::Pop, span.line);

                if let Some(idx_name) = index_name {
                    self.chunk().emit_op(Op::GetLocal, span.line);
                    self.chunk().emit_byte(i_slot, span.line);
                    self.add_local(idx_name);
                    let is = (self.locals.len() - 1) as u8;
                    self.chunk().emit_op(Op::SetLocal, span.line);
                    self.chunk().emit_byte(is, span.line);
                    self.chunk().emit_op(Op::Pop, span.line);
                }

                self.compile_block(body)?;

                if let Some(ctx) = self.loop_stack.last_mut() {
                    let conts = std::mem::take(&mut ctx.continues);
                    for c in conts {
                        self.chunk().patch_jump(c);
                    }
                }
                // i++
                self.chunk().emit_op(Op::IncLocal, span.line);
                self.chunk().emit_byte(i_slot, span.line);

                self.chunk().emit_loop(loop_start, span.line);
                self.chunk().patch_jump(exit);
                self.chunk().emit_op(Op::Pop, span.line);
                if let Some(ctx) = self.loop_stack.pop() {
                    for b in ctx.breaks {
                        self.chunk().patch_jump(b);
                    }
                }
                self.end_scope();
            }
            Stmt::Break(span) => {
                let jump = self.chunk().emit_jump(Op::Jump, span.line);
                if let Some(ctx) = self.loop_stack.last_mut() {
                    ctx.breaks.push(jump);
                } else {
                    return Err(CompileError::syntax("break outside loop", *span));
                }
            }
            Stmt::Continue(span) => {
                let jump = self.chunk().emit_jump(Op::Jump, span.line);
                if let Some(ctx) = self.loop_stack.last_mut() {
                    ctx.continues.push(jump);
                } else {
                    return Err(CompileError::syntax("continue outside loop", *span));
                }
            }
            Stmt::Block(b) => self.compile_block(b)?,
            Stmt::Switch { expr, cases, span } => {
                let ln = span.line as usize;
                self.compile_expr(expr)?;
                // Switch value stays on stack top throughout all case tests.
                let mut end_jumps: Vec<usize> = Vec::new();
                for case in cases {
                    if case.patterns.is_empty() {
                        // default arm
                        self.chunk().emit_op(Op::Pop, ln); // pop switch value
                        self.compile_switch_body(&case.body, &mut end_jumps, ln)?;
                    } else {
                        // One or more patterns: OR-them together.
                        let mut skip_jumps: Vec<usize> = Vec::new();
                        let mut next_jumps: Vec<usize> = Vec::new();

                        for (i, pat) in case.patterns.iter().enumerate() {
                            let is_last = i == case.patterns.len() - 1;
                            match pat {
                                crate::ast::SwitchPattern::Expr(e) => {
                                    self.chunk().emit_op(Op::Dup, ln);
                                    self.compile_expr(e)?;
                                    self.chunk().emit_op(Op::Eq, ln);
                                    if is_last {
                                        let nj = self.chunk().emit_jump(Op::JumpIfFalse, ln);
                                        next_jumps.push(nj);
                                        self.chunk().emit_op(Op::Pop, ln);
                                    } else {
                                        let sj = self.chunk().emit_jump(Op::JumpIfTrue, ln);
                                        skip_jumps.push(sj);
                                        self.chunk().emit_op(Op::Pop, ln);
                                    }
                                }
                                crate::ast::SwitchPattern::Range(lo, hi) => {
                                    self.chunk().emit_op(Op::Dup, ln);
                                    self.compile_expr(lo)?;
                                    self.chunk().emit_op(Op::Ge, ln);
                                    self.chunk().emit_op(Op::Dup, ln);
                                    let fail_lo = self.chunk().emit_jump(Op::JumpIfFalse, ln);
                                    self.chunk().emit_op(Op::Pop, ln);
                                    self.chunk().emit_op(Op::Dup, ln);
                                    self.compile_expr(hi)?;
                                    self.chunk().emit_op(Op::Le, ln);
                                    if is_last {
                                        let nj = self.chunk().emit_jump(Op::JumpIfFalse, ln);
                                        next_jumps.push(nj);
                                        self.chunk().emit_op(Op::Pop, ln);
                                    } else {
                                        let sj = self.chunk().emit_jump(Op::JumpIfTrue, ln);
                                        skip_jumps.push(sj);
                                        self.chunk().emit_op(Op::Pop, ln);
                                    }
                                    self.chunk().patch_jump(fail_lo);
                                    self.chunk().emit_op(Op::Pop, ln);
                                }
                            }
                        }

                        // Patch OR-match jumps → reach body
                        for sj in skip_jumps {
                            self.chunk().patch_jump(sj);
                            self.chunk().emit_op(Op::Pop, ln);
                        }

                        // Optional guard: `when <expr>`
                        let guard_fail: Option<usize> = if let Some(g) = &case.guard {
                            self.compile_expr(g)?;
                            Some(self.chunk().emit_jump(Op::JumpIfFalse, ln))
                        } else {
                            None
                        };

                        // Bind switch value to name if requested
                        if let Some(bind) = &case.pattern_bind {
                            self.chunk().emit_op(Op::Dup, ln);
                            let slot = self.locals.len() as u8;
                            self.add_local(bind);
                            self.chunk().emit_op(Op::SetLocal, ln);
                            self.chunk().emit_byte(slot, ln);
                            self.chunk().emit_op(Op::Pop, ln);
                        }

                        // Pop switch value, run body
                        self.chunk().emit_op(Op::Pop, ln);
                        self.compile_switch_body(&case.body, &mut end_jumps, ln)?;
                        let ej = self.chunk().emit_jump(Op::Jump, ln);
                        end_jumps.push(ej);

                        // Guard fail: pop guard result + switch value, skip to end
                        if let Some(gf) = guard_fail {
                            self.chunk().patch_jump(gf);
                            self.chunk().emit_op(Op::Pop, ln); // pop guard false
                            self.chunk().emit_op(Op::Pop, ln); // pop switch value
                            let ej2 = self.chunk().emit_jump(Op::Jump, ln);
                            end_jumps.push(ej2);
                        }

                        // Patch next-case jumps
                        for nj in next_jumps {
                            self.chunk().patch_jump(nj);
                            self.chunk().emit_op(Op::Pop, ln); // pop false
                        }
                    }
                }
                // Fall-through: pop switch value
                self.chunk().emit_op(Op::Pop, ln);
                for j in end_jumps {
                    self.chunk().patch_jump(j);
                }
            }
            Stmt::Throw(e, span) => {
                self.compile_expr(e)?;
                self.chunk().emit_op(Op::Throw, span.line);
            }
            Stmt::Try {
                body,
                catches,
                finally,
                span,
            } => {
                let try_begin = self.chunk().emit_jump(Op::TryBegin, span.line);
                self.compile_block(body)?;
                self.chunk().emit_op(Op::TryEnd, span.line);
                let end_jump = self.chunk().emit_jump(Op::Jump, span.line);
                self.chunk().patch_jump(try_begin);
                // catch handlers — simplified: first catch gets exception on stack
                if let Some(catch) = catches.first() {
                    if let Some(name) = &catch.name {
                        self.add_local(name);
                        let slot = (self.locals.len() - 1) as u8;
                        self.chunk().emit_op(Op::SetLocal, span.line);
                        self.chunk().emit_byte(slot, span.line);
                        self.chunk().emit_op(Op::Pop, span.line);
                    } else {
                        self.chunk().emit_op(Op::Pop, span.line);
                    }
                    self.compile_block(&catch.body)?;
                }
                self.chunk().patch_jump(end_jump);
                if let Some(f) = finally {
                    self.compile_block(f)?;
                }
            }
            Stmt::Using { decl, body, span } => {
                self.compile_stmt(&Stmt::Decl(decl.clone()))?;
                self.compile_block(body)?;
                // Call Dispose at end of using block
                if let Some(slot) = self.resolve_local(&decl.name) {
                    self.chunk().emit_op(Op::GetLocal, span.line);
                    self.chunk().emit_byte(slot, span.line);
                    self.chunk().emit_op(Op::Dup, span.line);
                    self.chunk()
                        .emit_constant(Value::String("Dispose".into()), span.line);
                    self.chunk().emit_op(Op::GetProperty, span.line);
                    self.chunk().emit_op(Op::Call, span.line);
                    self.chunk().emit_byte(1, span.line);
                    self.chunk().emit_op(Op::Pop, span.line);
                }
            }
            Stmt::Unsafe(body, _) => self.compile_block(body)?,
            Stmt::Match { expr, arms, span } => {
                self.compile_expr(expr)?;
                let mut end_jumps = Vec::new();
                for arm in arms {
                    self.chunk().emit_op(Op::Dup, span.line);
                    // Result: Ok → fields.ok == true; Error → ok == false
                    // Also support Tag string property for other ADTs
                    let pat = arm.pattern.as_str();
                    if pat == "Ok" || pat == "Error" || pat == "Err" {
                        self.chunk()
                            .emit_constant(Value::String("ok".into()), span.line);
                        self.chunk().emit_op(Op::GetProperty, span.line);
                        if pat == "Ok" {
                            self.chunk().emit_op(Op::True, span.line);
                        } else {
                            self.chunk().emit_op(Op::False, span.line);
                        }
                        self.chunk().emit_op(Op::Eq, span.line);
                    } else {
                        self.chunk()
                            .emit_constant(Value::String("Tag".into()), span.line);
                        self.chunk().emit_op(Op::GetProperty, span.line);
                        self.chunk()
                            .emit_constant(Value::String(arm.pattern.clone().into()), span.line);
                        self.chunk().emit_op(Op::Eq, span.line);
                    }
                    let next = self.chunk().emit_jump(Op::JumpIfFalse, span.line);
                    self.chunk().emit_op(Op::Pop, span.line);
                    if let Some(bind) = &arm.bind {
                        let field = if pat == "Error" || pat == "Err" {
                            "error"
                        } else {
                            "value"
                        };
                        self.chunk().emit_op(Op::Dup, span.line);
                        self.chunk()
                            .emit_constant(Value::String(field.into()), span.line);
                        self.chunk().emit_op(Op::GetProperty, span.line);
                        self.add_local(bind);
                        let slot = (self.locals.len() - 1) as u8;
                        self.chunk().emit_op(Op::SetLocal, span.line);
                        self.chunk().emit_byte(slot, span.line);
                        self.chunk().emit_op(Op::Pop, span.line);
                    }
                    self.chunk().emit_op(Op::Pop, span.line); // drop matched value
                    self.compile_expr(&arm.body)?;
                    self.chunk().emit_op(Op::Pop, span.line);
                    let end = self.chunk().emit_jump(Op::Jump, span.line);
                    end_jumps.push(end);
                    self.chunk().patch_jump(next);
                    self.chunk().emit_op(Op::Pop, span.line);
                }
                self.chunk().emit_op(Op::Pop, span.line);
                for j in end_jumps {
                    self.chunk().patch_jump(j);
                }
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> CompileResult<()> {
        let line = expr.span().line;
        match expr {
            Expr::Int(n, _) => self.chunk().emit_constant(Value::Int(*n), line),
            Expr::UInt(n, _) => self.chunk().emit_constant(Value::UInt(*n), line),
            Expr::Float(n, _) => self.chunk().emit_constant(Value::Float(*n), line),
            Expr::Decimal(s, _) => {
                let v: f64 = s.parse().unwrap_or(0.0);
                self.chunk().emit_constant(Value::Float(v), line);
            }
            Expr::Bool(true, _) => self.chunk().emit_op(Op::True, line),
            Expr::Bool(false, _) => self.chunk().emit_op(Op::False, line),
            Expr::Char(c, _) => self.chunk().emit_constant(Value::Char(*c), line),
            Expr::String(s, _) => self
                .chunk()
                .emit_constant(Value::String(Rc::<str>::from(s.as_str())), line),
            Expr::Null(_) => self.chunk().emit_op(Op::Null, line),
            Expr::Interpolated(parts, _) => {
                self.chunk()
                    .emit_constant(Value::String(Rc::<str>::from("")), line);
                for part in parts {
                    match part {
                        InterpPart::Literal(s) => {
                            self.chunk()
                                .emit_constant(Value::String(Rc::<str>::from(s.as_str())), line);
                            self.chunk().emit_op(Op::Add, line);
                        }
                        InterpPart::Expr(e) => {
                            self.compile_expr(e)?;
                            self.chunk().emit_op(Op::ToString, line);
                            self.chunk().emit_op(Op::Add, line);
                        }
                    }
                }
            }
            Expr::Ident(name, span) => {
                self.emit_get_name(name, span.line);
            }
            Expr::This(span) => {
                if let Some(slot) = self.resolve_local("this") {
                    self.chunk().emit_op(Op::GetLocal, span.line);
                    self.chunk().emit_byte(slot, span.line);
                } else {
                    return Err(CompileError::syntax("'this' outside method", *span));
                }
            }
            Expr::Base(span) => {
                if let Some(slot) = self.resolve_local("this") {
                    self.chunk().emit_op(Op::GetLocal, span.line);
                    self.chunk().emit_byte(slot, span.line);
                } else {
                    return Err(CompileError::syntax("'base' outside method", *span));
                }
            }
            Expr::Binary {
                left, op, right, ..
            } => {
                // Short-circuit for && and ||
                if *op == BinOp::And {
                    self.compile_expr(left)?;
                    let jump = self.chunk().emit_jump(Op::JumpIfFalse, line);
                    self.chunk().emit_op(Op::Pop, line);
                    self.compile_expr(right)?;
                    self.chunk().patch_jump(jump);
                    return Ok(());
                }
                if *op == BinOp::Or {
                    self.compile_expr(left)?;
                    let jump = self.chunk().emit_jump(Op::JumpIfTrue, line);
                    self.chunk().emit_op(Op::Pop, line);
                    self.compile_expr(right)?;
                    self.chunk().patch_jump(jump);
                    return Ok(());
                }
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                let opcode = match op {
                    BinOp::Add => Op::Add,
                    BinOp::Sub => Op::Sub,
                    BinOp::Mul => Op::Mul,
                    BinOp::Div => Op::Div,
                    BinOp::Mod => Op::Mod,
                    BinOp::Eq => Op::Eq,
                    BinOp::Ne => Op::Ne,
                    BinOp::Lt => Op::Lt,
                    BinOp::Le => Op::Le,
                    BinOp::Gt => Op::Gt,
                    BinOp::Ge => Op::Ge,
                    BinOp::BitAnd => Op::BitAnd,
                    BinOp::BitOr => Op::BitOr,
                    BinOp::BitXor => Op::BitXor,
                    BinOp::Shl => Op::Shl,
                    BinOp::Shr => Op::Shr,
                    BinOp::NullCoalesce => Op::NullCoalesce,
                    BinOp::And | BinOp::Or => unreachable!(),
                };
                self.chunk().emit_op(opcode, line);
            }
            Expr::Unary { op, expr, .. } => {
                match op {
                    UnOp::Neg => {
                        self.compile_expr(expr)?;
                        self.chunk().emit_op(Op::Neg, line);
                    }
                    UnOp::Not => {
                        self.compile_expr(expr)?;
                        self.chunk().emit_op(Op::Not, line);
                    }
                    UnOp::BitNot => {
                        self.compile_expr(expr)?;
                        self.chunk().emit_op(Op::BitNot, line);
                    }
                    UnOp::PreInc | UnOp::PostInc => {
                        if let Expr::Ident(name, _) = expr.as_ref() {
                            match self.resolve_name(name) {
                                NameRes::Local(slot) => {
                                    if *op == UnOp::PostInc {
                                        self.chunk().emit_op(Op::GetLocal, line);
                                        self.chunk().emit_byte(slot, line);
                                        self.chunk().emit_op(Op::IncLocal, line);
                                        self.chunk().emit_byte(slot, line);
                                    } else {
                                        self.chunk().emit_op(Op::IncLocal, line);
                                        self.chunk().emit_byte(slot, line);
                                        self.chunk().emit_op(Op::GetLocal, line);
                                        self.chunk().emit_byte(slot, line);
                                    }
                                }
                                NameRes::Upvalue(idx) => {
                                    self.chunk().emit_op(Op::GetUpvalue, line);
                                    self.chunk().emit_byte(idx, line);
                                    if *op == UnOp::PostInc {
                                        self.chunk().emit_op(Op::Dup, line);
                                    }
                                    self.chunk().emit_constant(Value::Int(1), line);
                                    self.chunk().emit_op(Op::Add, line);
                                    self.chunk().emit_op(Op::SetUpvalue, line);
                                    self.chunk().emit_byte(idx, line);
                                    if *op == UnOp::PostInc {
                                        self.chunk().emit_op(Op::Pop, line);
                                    }
                                }
                                NameRes::Global => {
                                    let g = self.ensure_global(name);
                                    self.chunk().emit_op(Op::GetGlobal, line);
                                    self.chunk().emit_byte(g, line);
                                    self.chunk().emit_constant(Value::Int(1), line);
                                    self.chunk().emit_op(Op::Add, line);
                                    self.chunk().emit_op(Op::SetGlobal, line);
                                    self.chunk().emit_byte(g, line);
                                }
                            }
                        } else {
                            self.compile_expr(expr)?;
                        }
                    }
                    UnOp::PreDec | UnOp::PostDec => {
                        if let Expr::Ident(name, _) = expr.as_ref() {
                            match self.resolve_name(name) {
                                NameRes::Local(slot) => {
                                    if *op == UnOp::PostDec {
                                        self.chunk().emit_op(Op::GetLocal, line);
                                        self.chunk().emit_byte(slot, line);
                                        self.chunk().emit_op(Op::DecLocal, line);
                                        self.chunk().emit_byte(slot, line);
                                    } else {
                                        self.chunk().emit_op(Op::DecLocal, line);
                                        self.chunk().emit_byte(slot, line);
                                        self.chunk().emit_op(Op::GetLocal, line);
                                        self.chunk().emit_byte(slot, line);
                                    }
                                }
                                NameRes::Upvalue(idx) => {
                                    self.chunk().emit_op(Op::GetUpvalue, line);
                                    self.chunk().emit_byte(idx, line);
                                    if *op == UnOp::PostDec {
                                        self.chunk().emit_op(Op::Dup, line);
                                    }
                                    self.chunk().emit_constant(Value::Int(1), line);
                                    self.chunk().emit_op(Op::Sub, line);
                                    self.chunk().emit_op(Op::SetUpvalue, line);
                                    self.chunk().emit_byte(idx, line);
                                    if *op == UnOp::PostDec {
                                        self.chunk().emit_op(Op::Pop, line);
                                    }
                                }
                                NameRes::Global => {
                                    let g = self.ensure_global(name);
                                    self.chunk().emit_op(Op::GetGlobal, line);
                                    self.chunk().emit_byte(g, line);
                                    self.chunk().emit_constant(Value::Int(1), line);
                                    self.chunk().emit_op(Op::Sub, line);
                                    self.chunk().emit_op(Op::SetGlobal, line);
                                    self.chunk().emit_byte(g, line);
                                }
                            }
                        } else {
                            self.compile_expr(expr)?;
                        }
                    }
                }
            }
            Expr::Assign {
                target, op, value, ..
            } => {
                match target.as_ref() {
                    Expr::Ident(name, _) => {
                        if *op == AssignOp::Assign {
                            self.compile_expr(value)?;
                        } else {
                            self.emit_get_name(name, line);
                            self.compile_expr(value)?;
                            self.emit_assign_binop(*op, line);
                        }
                        self.emit_set_name(name, line);
                    }
                    Expr::Member { object, field, .. } => {
                        self.compile_expr(object)?;
                        if *op == AssignOp::Assign {
                            self.compile_expr(value)?;
                        } else {
                            self.chunk().emit_op(Op::Dup, line);
                            self.chunk()
                                .emit_constant(Value::String(field.clone().into()), line);
                            self.chunk().emit_op(Op::GetProperty, line);
                            self.compile_expr(value)?;
                            self.emit_assign_binop(*op, line);
                        }
                        self.chunk()
                            .emit_constant(Value::String(field.clone().into()), line);
                        self.chunk().emit_op(Op::SetProperty, line);
                    }
                    Expr::Index {
                        object, indices, ..
                    } => {
                        self.compile_expr(object)?;
                        for idx in indices {
                            self.compile_expr(idx)?;
                        }
                        if *op != AssignOp::Assign {
                            // compound index assign not fully supported — just assign
                        }
                        self.compile_expr(value)?;
                        self.chunk().emit_op(Op::SetIndex, line);
                    }
                    _ => {
                        return Err(CompileError::syntax(
                            "invalid assignment target",
                            target.span(),
                        ));
                    }
                }
            }
            Expr::Call {
                callee, args, ..
            } => {
                // Special: print(...)
                if let Expr::Ident(name, _) = callee.as_ref() {
                    if name == "print" || name == "write" {
                        for a in args {
                            self.compile_expr(&a.value)?;
                        }
                        self.chunk().emit_op(Op::Print, line);
                        return Ok(());
                    }
                }
                // Method call: obj.method(args) => [fn, obj, args...]
                if let Expr::Member { object, field, .. } = callee.as_ref() {
                    if let Expr::Ident(type_name, _) = object.as_ref() {
                        if self.classes.contains_key(type_name) {
                            self.compile_expr(object)?;
                            self.chunk()
                                .emit_constant(Value::String(field.clone().into()), line);
                            self.chunk().emit_op(Op::GetProperty, line);
                            for a in args {
                                self.compile_expr(&a.value)?;
                            }
                            self.chunk().emit_op(Op::Call, line);
                            self.chunk().emit_byte(args.len() as u8, line);
                            return Ok(());
                        }
                    }
                    self.compile_expr(object)?;
                    self.chunk().emit_op(Op::Dup, line);
                    self.chunk()
                        .emit_constant(Value::String(field.clone().into()), line);
                    self.chunk().emit_op(Op::GetProperty, line);
                    // stack: [obj, method] — need [method, obj, args]
                    // Swap using: we'll Call with special arity where VM detects Object under Function
                    // Emit args then Call; VM handles [obj, method, args...] 
                    for a in args {
                        self.compile_expr(&a.value)?;
                    }
                    self.chunk().emit_op(Op::Call, line);
                    self.chunk().emit_byte((args.len() + 1) as u8, line); // +this
                    return Ok(());
                }
                self.compile_expr(callee)?;
                for a in args {
                    self.compile_expr(&a.value)?;
                }
                self.chunk().emit_op(Op::Call, line);
                self.chunk().emit_byte(args.len() as u8, line);
            }
            Expr::Member {
                object,
                field,
                null_safe,
                ..
            } => {
                self.compile_expr(object)?;
                if *null_safe {
                    self.chunk().emit_op(Op::Dup, line);
                    self.chunk().emit_op(Op::IsNull, line);
                    let jump = self.chunk().emit_jump(Op::JumpIfTrue, line);
                    self.chunk().emit_op(Op::Pop, line); // pop isnull false path's dup? 
                    // Simplified null-safe
                    self.chunk()
                        .emit_constant(Value::String(field.clone().into()), line);
                    self.chunk().emit_op(Op::GetProperty, line);
                    let end = self.chunk().emit_jump(Op::Jump, line);
                    self.chunk().patch_jump(jump);
                    self.chunk().emit_op(Op::Pop, line);
                    self.chunk().emit_op(Op::Null, line);
                    self.chunk().patch_jump(end);
                } else {
                    self.chunk()
                        .emit_constant(Value::String(field.clone().into()), line);
                    self.chunk().emit_op(Op::GetProperty, line);
                }
            }
            Expr::Index { object, indices, .. } => {
                self.compile_expr(object)?;
                for idx in indices {
                    self.compile_expr(idx)?;
                }
                self.chunk().emit_op(Op::GetIndex, line);
            }
            Expr::New { ty, args, init, .. } => {
                if ty.is_array || (!init.is_empty() && init.iter().all(|(n, _)| n.is_empty())) {
                    let count = init.len();
                    for (_, v) in init {
                        self.compile_expr(v)?;
                    }
                    self.chunk().emit_op(Op::NewArray, line);
                    self.chunk().emit_byte(count as u8, line);
                } else if ty.name == "List" {
                    let count = init.len();
                    for (_, v) in init {
                        self.compile_expr(v)?;
                    }
                    self.chunk().emit_op(Op::NewArray, line);
                    self.chunk().emit_byte(count as u8, line);
                } else if self.stdlib_enabled && ty.name == "Dictionary" {
                    self.chunk()
                        .emit_constant(crate::stdlib::new_dict(), line);
                    for (name, value) in init {
                        if name.is_empty() {
                            continue;
                        }
                        self.chunk().emit_op(Op::Dup, line);
                        self.chunk()
                            .emit_constant(Value::String(name.clone().into()), line);
                        self.compile_expr(value)?;
                        self.chunk().emit_op(Op::SetIndex, line);
                        self.chunk().emit_op(Op::Pop, line);
                    }
                } else if self.stdlib_enabled && matches!(ty.name.as_str(), "Set" | "Queue" | "Stack") {
                    self.chunk()
                        .emit_constant(crate::stdlib::new_collection(&ty.name), line);
                    for (_, value) in init {
                        let method = if ty.name == "Set" {
                            "Add"
                        } else if ty.name == "Queue" {
                            "Enqueue"
                        } else {
                            "Push"
                        };
                        self.chunk().emit_op(Op::Dup, line);
                        self.chunk()
                            .emit_constant(Value::String(method.into()), line);
                        self.chunk().emit_op(Op::GetProperty, line);
                        self.compile_expr(value)?;
                        self.chunk().emit_op(Op::Call, line);
                        self.chunk().emit_byte(2, line);
                        self.chunk().emit_op(Op::Pop, line);
                    }
                } else if self.stdlib_enabled && ty.name == "StringBuilder" {
                    self.chunk()
                        .emit_constant(crate::stdlib::new_string_builder(), line);
                } else if self.stdlib_enabled && ty.name == "Logger" {
                    self.chunk()
                        .emit_constant(crate::stdlib::new_logger(), line);
                } else if self.stdlib_enabled && ty.name == "TcpClient" {
                    self.chunk()
                        .emit_constant(crate::stdlib::new_tcp_client(), line);
                } else if self.stdlib_enabled && ty.name == "UdpSocket" {
                    self.chunk()
                        .emit_constant(crate::stdlib::new_udp_socket(0), line);
                    if let Some(a) = args.first() {
                        self.chunk().emit_op(Op::Dup, line);
                        self.compile_expr(&a.value)?;
                        self.chunk()
                            .emit_constant(Value::String("port".into()), line);
                        self.chunk().emit_op(Op::SetProperty, line);
                        self.chunk().emit_op(Op::Pop, line);
                    }
                } else if let Some(&ci) = self.classes.get(&ty.name) {
                    // [ctor?, obj, args...] then Call — or just obj if no ctor
                    if let Some(ctor_idx) = self.module.classes[ci].constructor {
                        self.chunk().emit_constant(
                            Value::Function(FunctionRef {
                                name: format!("{}.new", ty.name),
                                chunk_index: ctor_idx,
                                arity: args.len() + 1,
                                defaults: vec![],
                                is_async: false,
                                upvalues: vec![],
                            }),
                            line,
                        );
                    }
                    self.chunk().emit_op(Op::NewObject, line);
                    self.chunk().emit_byte(ci as u8, line);
                    for a in args {
                        self.compile_expr(&a.value)?;
                    }
                    if self.module.classes[ci].constructor.is_some() {
                        self.chunk().emit_op(Op::Call, line);
                        self.chunk().emit_byte((args.len() + 1) as u8, line);
                    }
                    for (name, value) in init {
                        if name.is_empty() {
                            continue;
                        }
                        self.chunk().emit_op(Op::Dup, line);
                        self.compile_expr(value)?;
                        self.chunk()
                            .emit_constant(Value::String(name.clone().into()), line);
                        self.chunk().emit_op(Op::SetProperty, line);
                        self.chunk().emit_op(Op::Pop, line);
                    }
                } else {
                    self.chunk().emit_op(Op::NewObject, line);
                    self.chunk().emit_byte(0xff, line);
                    self.chunk()
                        .emit_constant(Value::String(ty.name.clone().into()), line);
                }
            }
            Expr::ArrayLit(elems, _) => {
                for e in elems {
                    self.compile_expr(e)?;
                }
                self.chunk().emit_op(Op::NewArray, line);
                self.chunk().emit_byte(elems.len() as u8, line);
            }
            Expr::Ternary {
                cond,
                then_expr,
                else_expr,
                ..
            } => {
                self.compile_expr(cond)?;
                let else_jump = self.chunk().emit_jump(Op::JumpIfFalse, line);
                self.chunk().emit_op(Op::Pop, line);
                self.compile_expr(then_expr)?;
                let end = self.chunk().emit_jump(Op::Jump, line);
                self.chunk().patch_jump(else_jump);
                self.chunk().emit_op(Op::Pop, line);
                self.compile_expr(else_expr)?;
                self.chunk().patch_jump(end);
            }
            Expr::Grouped(e, _) => self.compile_expr(e)?,
            Expr::Cast { expr, .. } => self.compile_expr(expr)?,
            Expr::Await(e, _) => {
                self.compile_expr(e)?;
                self.chunk().emit_op(Op::Await, line);
            }
            Expr::Lambda { params, body, .. } => {
                let idx = self.module.chunks.len();
                let mut chunk = Chunk::new(format!("lambda_{}", idx));
                chunk.arity = params.len();
                self.module.chunks.push(chunk);
                let prev = self.current;
                self.current = idx;
                self.enclosing.push(EnclosingFn {
                    locals: std::mem::take(&mut self.locals),
                    local_ranges: std::mem::take(&mut self.local_ranges),
                    upvalues: std::mem::take(&mut self.upvalues),
                });
                let saved_depth = self.scope_depth;
                let saved_max = self.max_locals;
                self.scope_depth = 0;
                self.max_locals = 0;
                self.begin_scope();
                for p in params {
                    self.add_local(&p.name);
                }
                match body {
                    FunctionBody::Block(b) => self.compile_block(b)?,
                    FunctionBody::Expr(e) => {
                        self.compile_expr(e)?;
                        self.chunk().emit_op(Op::Return, line);
                    }
                }
                self.chunk().emit_op(Op::Null, line);
                self.chunk().emit_op(Op::Return, line);
                self.end_scope();
                self.finish_locals(idx);
                let lambda_upvalues = std::mem::take(&mut self.upvalues);
                let enc = self.enclosing.pop().expect("enclosing lambda frame");
                self.locals = enc.locals;
                self.local_ranges = enc.local_ranges;
                self.upvalues = enc.upvalues;
                self.scope_depth = saved_depth;
                self.max_locals = saved_max;
                self.current = prev;
                self.chunk().emit_constant(
                    Value::Function(FunctionRef {
                        name: format!("lambda_{}", idx),
                        chunk_index: idx,
                        arity: params.len(),
                        defaults: vec![],
                        is_async: false,
                        upvalues: vec![],
                    }),
                    line,
                );
                if !lambda_upvalues.is_empty() {
                    self.chunk().emit_op(Op::MakeClosure, line);
                    self.chunk()
                        .emit_byte(lambda_upvalues.len() as u8, line);
                    for uv in &lambda_upvalues {
                        self.chunk()
                            .emit_byte(if uv.is_local { 1 } else { 0 }, line);
                        self.chunk().emit_byte(uv.index, line);
                    }
                }
            }
            Expr::Is { expr, .. } => {
                self.compile_expr(expr)?;
                // Simplified: always leave bool based on non-null
                self.chunk().emit_op(Op::IsNull, line);
                self.chunk().emit_op(Op::Not, line);
            }
            Expr::As { expr, .. } => self.compile_expr(expr)?,
            Expr::TypeOf(_, _) => {
                self.chunk()
                    .emit_constant(Value::String("Type".into()), line);
            }
            Expr::Deref(e, _) | Expr::AddressOf(e, _) | Expr::Try(e, _) => {
                self.compile_expr(e)?;
            }
            Expr::PtrMember { object, field, .. } => {
                self.compile_expr(object)?;
                self.chunk()
                    .emit_constant(Value::String(field.clone().into()), line);
                self.chunk().emit_op(Op::GetProperty, line);
            }
        }
        Ok(())
    }

    fn emit_assign_binop(&mut self, op: AssignOp, line: usize) {
        let opcode = match op {
            AssignOp::Add => Op::Add,
            AssignOp::Sub => Op::Sub,
            AssignOp::Mul => Op::Mul,
            AssignOp::Div => Op::Div,
            AssignOp::Mod => Op::Mod,
            AssignOp::BitAnd => Op::BitAnd,
            AssignOp::BitOr => Op::BitOr,
            AssignOp::BitXor => Op::BitXor,
            AssignOp::Shl => Op::Shl,
            AssignOp::Shr => Op::Shr,
            AssignOp::NullCoalesce => Op::NullCoalesce,
            AssignOp::Assign => return,
        };
        self.chunk().emit_op(opcode, line);
    }
}

fn peel_attributes(item: &Item) -> (Vec<&Attribute>, &Item) {
    match item {
        Item::Attribute(attr, inner) => {
            let (mut rest, core) = peel_attributes(inner);
            rest.insert(0, attr);
            (rest, core)
        }
        other => (Vec::new(), other),
    }
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}
