//! Semantic analysis and type checking for RayTask.

use crate::ast::*;
use crate::error::{CompileError, CompileResult};
use crate::span::Span;
use crate::types::{builtin_functions, ty_from_ref, Ty, TypeCategory};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct TypeCheckReport {
    pub errors: Vec<CompileError>,
    pub warnings: Vec<CompileError>,
}

impl TypeCheckReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn into_result(self) -> CompileResult<()> {
        if let Some(e) = self.errors.into_iter().next() {
            Err(e)
        } else {
            Ok(())
        }
    }

    /// Format all diagnostics for CLI display.
    pub fn format_all(&self) -> String {
        let mut out = String::new();
        for e in &self.errors {
            out.push_str(&format!("error: {}\n", e));
        }
        for w in &self.warnings {
            out.push_str(&format!("warning: {}\n", w));
        }
        out
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FuncSig {
    #[allow(dead_code)]
    pub name: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub type_params: Vec<String>,
    #[allow(dead_code)]
    pub is_method: bool,
    #[allow(dead_code)]
    pub is_static: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub(crate) struct TypeDef {
    #[allow(dead_code)]
    pub name: String,
    pub kind: TypeDefKind,
    pub type_params: Vec<String>,
    pub fields: HashMap<String, Ty>,
    pub static_fields: HashMap<String, Ty>,
    pub properties: HashMap<String, Ty>,
    pub static_properties: HashMap<String, Ty>,
    pub methods: HashMap<String, FuncSig>,
    pub constructors: Vec<FuncSig>,
    pub bases: Vec<String>,
    pub is_abstract: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeDefKind {
    Class,
    Struct,
    Interface,
}

#[derive(Debug, Clone)]
struct ScopeVar {
    ty: Ty,
    is_const: bool,
    #[allow(dead_code)]
    span: Span,
}

pub struct TypeChecker {
    errors: Vec<CompileError>,
    warnings: Vec<CompileError>,
    types: HashMap<String, TypeDef>,
    functions: HashMap<String, FuncSig>,
    globals: HashMap<String, ScopeVar>,
    scopes: Vec<HashMap<String, ScopeVar>>,
    /// Current function return type
    expected_return: Option<Ty>,
    /// Current `this` type
    this_ty: Option<Ty>,
    /// Current `base` type
    base_ty: Option<Ty>,
    /// Active generic type params
    type_params: HashSet<String>,
    /// Inheritance: child → parents
    inheritance: HashMap<String, Vec<String>>,
    /// Generic constraints declared on named types.
    generic_constraints: HashMap<String, Vec<GenericConstraint>>,
    /// Extension methods: receiver type name → method name → sig (params include `this`)
    extensions: HashMap<String, HashMap<String, FuncSig>>,
    /// Type currently being defined (allows self-references in members).
    current_type: Option<String>,
    in_loop: usize,
    in_unsafe: bool,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self::with_stdlib(true)
    }

    pub fn with_stdlib(stdlib_enabled: bool) -> Self {
        let mut tc = Self {
            errors: Vec::new(),
            warnings: Vec::new(),
            types: HashMap::new(),
            functions: HashMap::new(),
            globals: HashMap::new(),
            scopes: Vec::new(),
            expected_return: None,
            this_ty: None,
            base_ty: None,
            type_params: HashSet::new(),
            inheritance: HashMap::new(),
            generic_constraints: HashMap::new(),
            extensions: HashMap::new(),
            current_type: None,
            in_loop: 0,
            in_unsafe: false,
        };
        if stdlib_enabled {
            for (name, params, ret) in builtin_functions() {
                let params: Vec<_> = params
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| (format!("a{}", i), t))
                    .collect();
                tc.functions.insert(
                    name.to_string(),
                    FuncSig {
                        name: name.to_string(),
                        params,
                        ret,
                        type_params: vec![],
                        is_method: false,
                        is_static: true,
                        span: Span::default(),
                    },
                );
            }
            // Built-in collection types (stubs)
            tc.register_builtin_collections(true);
        }
        tc
    }

    fn register_builtin_collections(&mut self, stdlib_enabled: bool) {
        let mut list = TypeDef {
            name: "List".into(),
            kind: TypeDefKind::Class,
            type_params: vec!["T".into()],
            fields: HashMap::new(),
            static_fields: HashMap::new(),
            properties: HashMap::from([("Count".into(), Ty::Int), ("Length".into(), Ty::Int)]),
            static_properties: HashMap::new(),
            methods: HashMap::new(),
            constructors: vec![FuncSig {
                name: "new".into(),
                params: vec![],
                ret: Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::TypeParam("T".into())],
                },
                type_params: vec!["T".into()],
                is_method: false,
                is_static: true,
                span: Span::default(),
            }],
            bases: vec![],
            is_abstract: false,
            span: Span::default(),
        };
        list.methods.insert(
            "Add".into(),
            FuncSig {
                name: "Add".into(),
                params: vec![("item".into(), Ty::TypeParam("T".into()))],
                ret: Ty::Void,
                type_params: vec![],
                is_method: true,
                is_static: false,
                span: Span::default(),
            },
        );
        list.methods.insert(
            "Get".into(),
            FuncSig {
                name: "Get".into(),
                params: vec![("index".into(), Ty::Int)],
                ret: Ty::TypeParam("T".into()),
                type_params: vec![],
                is_method: true,
                is_static: false,
                span: Span::default(),
            },
        );
        list.methods.insert(
            "Contains".into(),
            FuncSig {
                name: "Contains".into(),
                params: vec![("item".into(), Ty::TypeParam("T".into()))],
                ret: Ty::Bool,
                type_params: vec![],
                is_method: true,
                is_static: false,
                span: Span::default(),
            },
        );
        list.methods.insert(
            "RemoveAt".into(),
            FuncSig {
                name: "RemoveAt".into(),
                params: vec![("index".into(), Ty::Int)],
                ret: Ty::Void,
                type_params: vec![],
                is_method: true,
                is_static: false,
                span: Span::default(),
            },
        );
        for (name, params, ret) in [
            (
                "Distinct",
                vec![],
                Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::TypeParam("T".into())],
                },
            ),
            (
                "Sort",
                vec![],
                Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::TypeParam("T".into())],
                },
            ),
            (
                "SortDesc",
                vec![],
                Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::TypeParam("T".into())],
                },
            ),
            (
                "Reverse",
                vec![],
                Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::TypeParam("T".into())],
                },
            ),
            (
                "Take",
                vec![("count".into(), Ty::Int)],
                Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::TypeParam("T".into())],
                },
            ),
            (
                "Skip",
                vec![("count".into(), Ty::Int)],
                Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::TypeParam("T".into())],
                },
            ),
            (
                "IndexOf",
                vec![("item".into(), Ty::TypeParam("T".into()))],
                Ty::Int,
            ),
            (
                "Copy",
                vec![],
                Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::TypeParam("T".into())],
                },
            ),
            (
                "Chunk",
                vec![("size".into(), Ty::Int)],
                Ty::Array {
                    elem: Box::new(Ty::Array {
                        elem: Box::new(Ty::TypeParam("T".into())),
                        dims: 1,
                    }),
                    dims: 1,
                },
            ),
            (
                "Range",
                vec![("start_or_count".into(), Ty::Int), ("count".into(), Ty::Int)],
                Ty::Array {
                    elem: Box::new(Ty::Int),
                    dims: 1,
                },
            ),
            (
                "Fill",
                vec![("value".into(), Ty::Dyn), ("count".into(), Ty::Int)],
                Ty::Array {
                    elem: Box::new(Ty::Dyn),
                    dims: 1,
                },
            ),
        ] {
            list.methods.insert(
                name.into(),
                FuncSig {
                    name: name.into(),
                    params,
                    ret,
                    type_params: vec![],
                    is_method: true,
                    is_static: false,
                    span: Span::default(),
                },
            );
        }
        self.types.insert("List".into(), list);

        // string members as a pseudo-type for member lookup
        let mut string_td = TypeDef {
            name: "string".into(),
            kind: TypeDefKind::Class,
            type_params: vec![],
            fields: HashMap::new(),
            static_fields: HashMap::new(),
            properties: HashMap::from([("Length".into(), Ty::Int)]),
            static_properties: HashMap::new(),
            methods: HashMap::new(),
            constructors: vec![],
            bases: vec![],
            is_abstract: false,
            span: Span::default(),
        };
        for (name, ret) in [
            ("ToUpper", Ty::String),
            ("ToLower", Ty::String),
            ("Trim", Ty::String),
            ("TrimStart", Ty::String),
            ("TrimEnd", Ty::String),
        ] {
            string_td.methods.insert(
                name.into(),
                FuncSig {
                    name: name.into(),
                    params: vec![],
                    ret,
                    type_params: vec![],
                    is_method: true,
                    is_static: false,
                    span: Span::default(),
                },
            );
        }
        string_td.methods.insert(
            "Contains".into(),
            FuncSig {
                name: "Contains".into(),
                params: vec![("s".into(), Ty::String)],
                ret: Ty::Bool,
                type_params: vec![],
                is_method: true,
                is_static: false,
                span: Span::default(),
            },
        );
        string_td.methods.insert(
            "StartsWith".into(),
            FuncSig {
                name: "StartsWith".into(),
                params: vec![("s".into(), Ty::String)],
                ret: Ty::Bool,
                type_params: vec![],
                is_method: true,
                is_static: false,
                span: Span::default(),
            },
        );
        string_td.methods.insert(
            "EndsWith".into(),
            FuncSig {
                name: "EndsWith".into(),
                params: vec![("s".into(), Ty::String)],
                ret: Ty::Bool,
                type_params: vec![],
                is_method: true,
                is_static: false,
                span: Span::default(),
            },
        );
        string_td.methods.insert(
            "Substring".into(),
            FuncSig {
                name: "Substring".into(),
                params: vec![("start".into(), Ty::Int), ("len".into(), Ty::Int)],
                ret: Ty::String,
                type_params: vec![],
                is_method: true,
                is_static: false,
                span: Span::default(),
            },
        );
        string_td.methods.insert(
            "Replace".into(),
            FuncSig {
                name: "Replace".into(),
                params: vec![("a".into(), Ty::String), ("b".into(), Ty::String)],
                ret: Ty::String,
                type_params: vec![],
                is_method: true,
                is_static: false,
                span: Span::default(),
            },
        );
        for (name, params, ret) in [
            ("PadLeft", vec![("width".into(), Ty::Int), ("ch".into(), Ty::String)], Ty::String),
            ("PadRight", vec![("width".into(), Ty::Int), ("ch".into(), Ty::String)], Ty::String),
            ("Repeat", vec![("count".into(), Ty::Int)], Ty::String),
            ("Reverse", vec![], Ty::String),
            (
                "Chars",
                vec![],
                Ty::Array {
                    elem: Box::new(Ty::Char),
                    dims: 1,
                },
            ),
            (
                "Lines",
                vec![],
                Ty::Array {
                    elem: Box::new(Ty::String),
                    dims: 1,
                },
            ),
            ("ParseInt", vec![], Ty::Int),
            ("ParseFloat", vec![], Ty::Double),
            ("IsEmpty", vec![], Ty::Bool),
            ("IsWhitespace", vec![], Ty::Bool),
            ("Count", vec![("sub".into(), Ty::String)], Ty::Int),
            ("Remove", vec![("index".into(), Ty::Int), ("count".into(), Ty::Int)], Ty::String),
            ("Insert", vec![("index".into(), Ty::Int), ("text".into(), Ty::String)], Ty::String),
        ] {
            string_td.methods.insert(
                name.into(),
                FuncSig {
                    name: name.into(),
                    params,
                    ret,
                    type_params: vec![],
                    is_method: true,
                    is_static: false,
                    span: Span::default(),
                },
            );
        }
        self.types.insert("string".into(), string_td);

        // int.Parse etc.
        for prim in ["int", "float", "double", "long", "bool"] {
            let mut td = TypeDef {
                name: prim.into(),
                kind: TypeDefKind::Struct,
                type_params: vec![],
                fields: HashMap::new(),
                static_fields: HashMap::new(),
                properties: HashMap::new(),
                static_properties: HashMap::new(),
                methods: HashMap::new(),
                constructors: vec![],
                bases: vec![],
                is_abstract: false,
                span: Span::default(),
            };
            let ret = ty_from_name(prim);
            td.methods.insert(
                "Parse".into(),
                FuncSig {
                    name: "Parse".into(),
                    params: vec![("s".into(), Ty::String)],
                    ret,
                    type_params: vec![],
                    is_method: false,
                    is_static: true,
                    span: Span::default(),
                },
            );
            self.types.insert(prim.into(), td);
        }

        if stdlib_enabled {
        if stdlib_enabled {
            crate::stdlib_types::register_into(&mut self.types);
        }
        }
    }

    pub fn check(mut self, program: &Program) -> TypeCheckReport {
        // Pass 1a: register empty type shells (for forward references)
        for item in &program.items {
            self.predeclare_item(item);
        }
        // Pass 1b: fill members / functions / consts
        for item in &program.items {
            self.collect_item(item);
        }
        // Pass 2: resolve inheritance / overrides
        self.check_inheritance();
        // Pass 3: check bodies
        for item in &program.items {
            self.check_item(item);
        }

        TypeCheckReport {
            errors: self.errors,
            warnings: self.warnings,
        }
    }

    fn predeclare_item(&mut self, item: &Item) {
        match item {
            Item::Attribute(_, inner) => self.predeclare_item(inner),
            Item::Namespace(ns) => {
                for i in &ns.items {
                    self.predeclare_item(i);
                }
            }
            Item::Class(c) => {
                if self.types.contains_key(&c.name) {
                    return;
                }
                self.types.insert(
                    c.name.clone(),
                    TypeDef {
                        name: c.name.clone(),
                        kind: TypeDefKind::Class,
                        type_params: c.type_params.clone(),
                        fields: HashMap::new(),
                        static_fields: HashMap::new(),
                        properties: HashMap::new(),
                        static_properties: HashMap::new(),
                        methods: HashMap::new(),
                        constructors: vec![],
                        bases: c.bases.iter().map(|b| b.name.clone()).collect(),
                        is_abstract: c.is_abstract,
                        span: c.span,
                    },
                );
                self.inheritance.insert(
                    c.name.clone(),
                    c.bases.iter().map(|b| b.name.clone()).collect(),
                );
                self.generic_constraints
                    .insert(c.name.clone(), c.constraints.clone());
            }
            Item::Struct(s) => {
                if self.types.contains_key(&s.name) {
                    return;
                }
                self.types.insert(
                    s.name.clone(),
                    TypeDef {
                        name: s.name.clone(),
                        kind: TypeDefKind::Struct,
                        type_params: s.type_params.clone(),
                        fields: HashMap::new(),
                        static_fields: HashMap::new(),
                        properties: HashMap::new(),
                        static_properties: HashMap::new(),
                        methods: HashMap::new(),
                        constructors: vec![],
                        bases: vec![],
                        is_abstract: false,
                        span: s.span,
                    },
                );
                self.generic_constraints.insert(s.name.clone(), Vec::new());
            }
            Item::Interface(i) => {
                if self.types.contains_key(&i.name) {
                    return;
                }
                self.types.insert(
                    i.name.clone(),
                    TypeDef {
                        name: i.name.clone(),
                        kind: TypeDefKind::Interface,
                        type_params: i.type_params.clone(),
                        fields: HashMap::new(),
                        static_fields: HashMap::new(),
                        properties: HashMap::new(),
                        static_properties: HashMap::new(),
                        methods: HashMap::new(),
                        constructors: vec![],
                        bases: vec![],
                        is_abstract: true,
                        span: i.span,
                    },
                );
                self.generic_constraints.insert(i.name.clone(), Vec::new());
            }
            _ => {}
        }
    }

    fn err(&mut self, message: impl Into<String>, span: Span) {
        self.errors.push(CompileError::type_err(message, span));
    }

    fn warn(&mut self, message: impl Into<String>, span: Span) {
        self.warnings.push(CompileError::type_err(message, span));
    }

    fn resolve_msg(&mut self, message: impl Into<String>, span: Span) {
        self.errors.push(CompileError::resolve(message, span));
    }

    // ─── Collection ──────────────────────────────────────────

    fn collect_item(&mut self, item: &Item) {
        match item {
            Item::Attribute(_, inner) => self.collect_item(inner),
            Item::Namespace(ns) => {
                for i in &ns.items {
                    self.collect_item(i);
                }
            }
            Item::Class(c) => self.collect_class(c),
            Item::Struct(s) => self.collect_struct(s),
            Item::Union(u) => {
                // Register like a struct type for name resolution.
                self.collect_struct(&StructDecl {
                    access: u.access.clone(),
                    name: u.name.clone(),
                    type_params: vec![],
                    members: u.members.clone(),
                    attributes: u.attributes.clone(),
                    packed: u.packed,
                    align: u.align,
                    repr_c: true,
                    span: u.span,
                });
            }
            Item::Interface(i) => self.collect_interface(i),
            Item::Function(f) => self.collect_function(f, false),
            Item::Const(c) => {
                let ty = self.resolve_type_ref(&c.ty);
                self.globals.insert(
                    c.name.clone(),
                    ScopeVar {
                        ty,
                        is_const: true,
                        span: c.span,
                    },
                );
            }
            Item::Import(_) | Item::Module(_) => {}
        }
    }

    fn collect_class(&mut self, c: &ClassDecl) {
        if !self.types.contains_key(&c.name) {
            self.predeclare_item(&Item::Class(c.clone()));
        }
        let mut def = self.types.get(&c.name).cloned().expect("class predeclared");
        def.fields.clear();
        def.static_fields.clear();
        def.properties.clear();
        def.static_properties.clear();
        def.methods.clear();
        def.constructors.clear();
        def.bases = c.bases.iter().map(|b| b.name.clone()).collect();
        def.is_abstract = c.is_abstract;
        self.inheritance
            .insert(c.name.clone(), def.bases.clone());

        let saved = self.type_params.clone();
        for tp in &c.type_params {
            self.type_params.insert(tp.clone());
        }
        self.current_type = Some(c.name.clone());
        for m in &c.members {
            self.collect_member(&mut def, m, &c.name);
        }
        self.current_type = None;
        self.type_params = saved;
        self.types.insert(c.name.clone(), def);
    }

    fn collect_struct(&mut self, s: &StructDecl) {
        if !self.types.contains_key(&s.name) {
            self.predeclare_item(&Item::Struct(s.clone()));
        }
        let mut def = self.types.get(&s.name).cloned().expect("struct predeclared");
        def.fields.clear();
        def.static_fields.clear();
        def.properties.clear();
        def.static_properties.clear();
        def.methods.clear();
        def.constructors.clear();

        let saved = self.type_params.clone();
        for tp in &s.type_params {
            self.type_params.insert(tp.clone());
        }
        self.current_type = Some(s.name.clone());
        for m in &s.members {
            self.collect_member(&mut def, m, &s.name);
        }
        self.current_type = None;
        self.type_params = saved;
        self.types.insert(s.name.clone(), def);
    }

    fn collect_interface(&mut self, i: &InterfaceDecl) {
        if !self.types.contains_key(&i.name) {
            self.predeclare_item(&Item::Interface(i.clone()));
        }
        let mut def = self.types.get(&i.name).cloned().expect("interface predeclared");
        def.fields.clear();
        def.static_fields.clear();
        def.properties.clear();
        def.static_properties.clear();
        def.methods.clear();

        let saved = self.type_params.clone();
        for tp in &i.type_params {
            self.type_params.insert(tp.clone());
        }
        self.current_type = Some(i.name.clone());
        for m in &i.members {
            self.collect_member(&mut def, m, &i.name);
        }
        self.current_type = None;
        self.type_params = saved;
        self.types.insert(i.name.clone(), def);
    }

    fn collect_member(&mut self, def: &mut TypeDef, m: &Member, type_name: &str) {
        match m {
            Member::Field(f) => {
                let ty = f
                    .ty
                    .as_ref()
                    .map(|t| self.resolve_type_ref(t))
                    .unwrap_or(Ty::Dyn);
                let target = if f.is_static {
                    &mut def.static_fields
                } else {
                    &mut def.fields
                };
                if target.contains_key(&f.name) {
                    self.err(
                        format!("duplicate field '{}.{}'", type_name, f.name),
                        f.span,
                    );
                }
                target.insert(f.name.clone(), ty);
            }
            Member::Property(p) => {
                let ty = self.resolve_type_ref(&p.ty);
                if p.is_static {
                    def.static_properties.insert(p.name.clone(), ty.clone());
                } else {
                    def.properties.insert(p.name.clone(), ty.clone());
                }
                // auto-props also act as fields
                if p.auto {
                    if p.is_static {
                        def.static_fields.entry(p.name.clone()).or_insert(ty);
                    } else {
                        def.fields.entry(p.name.clone()).or_insert(ty);
                    }
                }
            }
            Member::Method(f) => {
                let sig = self.func_sig_from(f, true);
                if def.methods.contains_key(&f.name) {
                    self.err(
                        format!("duplicate method '{}.{}'", type_name, f.name),
                        f.span,
                    );
                }
                def.methods.insert(f.name.clone(), sig);
            }
            Member::Constructor(c) => {
                let params: Vec<_> = c
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), self.resolve_type_ref(&p.ty)))
                    .collect();
                def.constructors.push(FuncSig {
                    name: "new".into(),
                    params,
                    ret: Ty::Named(type_name.into()),
                    type_params: def.type_params.clone(),
                    is_method: false,
                    is_static: true,
                    span: c.span,
                });
            }
            Member::Destructor(_) | Member::Operator(_) => {}
            Member::Indexer(idx) => {
                // Expose get_Item / set_Item for member typing; index expr uses check_index
                let params: Vec<_> = idx
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), self.resolve_type_ref(&p.ty)))
                    .collect();
                let ret = self.resolve_type_ref(&idx.ty);
                def.methods.insert(
                    "get_Item".into(),
                    FuncSig {
                        name: "get_Item".into(),
                        params: params.clone(),
                        ret: ret.clone(),
                        type_params: vec![],
                        is_method: true,
                        is_static: false,
                        span: idx.span,
                    },
                );
                if idx.setter.is_some() {
                    let mut set_params = params;
                    set_params.push(("value".into(), ret));
                    def.methods.insert(
                        "set_Item".into(),
                        FuncSig {
                            name: "set_Item".into(),
                            params: set_params,
                            ret: Ty::Void,
                            type_params: vec![],
                            is_method: true,
                            is_static: false,
                            span: idx.span,
                        },
                    );
                }
            }
        }
    }

    fn collect_function(&mut self, f: &FunctionDecl, is_method: bool) {
        let sig = self.func_sig_from(f, is_method);
        if is_method {
            return;
        }
        if f.is_extension {
            if let Some(p) = f.params.first() {
                let recv = p.ty.name.clone();
                self.extensions
                    .entry(recv)
                    .or_default()
                    .insert(f.name.clone(), sig.clone());
            }
        }
        if is_builtin_name(&f.name) {
            self.warn(
                format!("function '{}' shadows a builtin", f.name),
                f.span,
            );
        } else if self.functions.contains_key(&f.name) {
            self.err(format!("duplicate function '{}'", f.name), f.span);
        }
        self.functions.insert(f.name.clone(), sig);
    }

    fn func_sig_from(&mut self, f: &FunctionDecl, is_method: bool) -> FuncSig {
        let saved = self.type_params.clone();
        for tp in &f.type_params {
            self.type_params.insert(tp.clone());
        }
        let params: Vec<_> = f
            .params
            .iter()
            .map(|p| (p.name.clone(), self.resolve_type_ref(&p.ty)))
            .collect();
        let declared = self.resolve_type_ref(&f.return_type);
        let ret = if f.is_async {
            wrap_task_return(declared)
        } else {
            declared
        };
        self.type_params = saved;
        FuncSig {
            name: f.name.clone(),
            params,
            ret,
            type_params: f.type_params.clone(),
            is_method,
            is_static: f.is_static,
            span: f.span,
        }
    }

    fn resolve_type_ref(&mut self, tr: &TypeRef) -> Ty {
        // Self / type currently being defined
        if self.current_type.as_ref() == Some(&tr.name) && tr.args.is_empty() {
            let mut t = Ty::Named(tr.name.clone());
            if tr.is_array {
                t = Ty::Array {
                    elem: Box::new(t),
                    dims: tr.array_dims.max(1),
                };
            }
            if tr.nullable {
                t = t.make_nullable();
            }
            return t;
        }
        // Type params
        if self.type_params.contains(&tr.name) && tr.args.is_empty() && !tr.is_array {
            let mut t = Ty::TypeParam(tr.name.clone());
            if tr.nullable {
                t = t.make_nullable();
            }
            return t;
        }
        let ty = ty_from_ref(tr);
        // Validate named types exist
        match &ty {
            Ty::Named(n) => {
                if !is_primitive_name(n)
                    && !self.types.contains_key(n)
                    && !self.type_params.contains(n)
                    && self.current_type.as_ref() != Some(n)
                {
                    self.err(format!("unknown type '{}'", n), tr.span);
                    return Ty::Error;
                }
            }
            Ty::Generic { name, .. } => {
                if !self.types.contains_key(name) && name != "ptr" {
                    if !matches!(
                        name.as_str(),
                        "List"
                            | "Dictionary"
                            | "Set"
                            | "Queue"
                            | "Stack"
                            | "Task"
                            | "Result"
                            | "ptr"
                    ) {
                        self.warn(format!("unknown generic type '{}'", name), tr.span);
                    }
                }
                self.validate_named_generic_instantiation(name, tr, &ty);
            }
            _ => {}
        }
        ty
    }

    fn validate_named_generic_instantiation(&mut self, name: &str, tr: &TypeRef, ty: &Ty) {
        let Ty::Generic { args, .. } = ty else {
            return;
        };
        let Some(td) = self.types.get(name).cloned() else {
            return;
        };
        if args.len() != td.type_params.len() {
            self.err(
                format!(
                    "generic type '{}' expects {} type argument(s), found {}",
                    name,
                    td.type_params.len(),
                    args.len()
                ),
                tr.span,
            );
            return;
        }
        let constraints = self
            .generic_constraints
            .get(name)
            .cloned()
            .unwrap_or_default();
        for gc in constraints {
            let Some(pos) = td.type_params.iter().position(|tp| tp == &gc.type_param) else {
                continue;
            };
            let Some(arg) = args.get(pos) else {
                continue;
            };
            for bound in gc.bounds {
                if bound.name == "new" && bound.args.is_empty() {
                    if !self.type_has_parameterless_constructor(arg) {
                        self.err(
                            format!(
                                "type argument '{}' for '{}' must satisfy 'new()'",
                                arg, gc.type_param
                            ),
                            tr.span,
                        );
                    }
                    continue;
                }
                let expected = self.resolve_type_ref(&bound);
                if !self.is_semantically_assignable(arg, &expected) {
                    self.err(
                        format!(
                            "type argument '{}' for '{}' must satisfy '{}'",
                            arg, gc.type_param, expected
                        ),
                        tr.span,
                    );
                }
            }
        }
    }

    fn type_has_parameterless_constructor(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Named(name) | Ty::Generic { name, .. } => self
                .types
                .get(name)
                .map(|td| td.kind == TypeDefKind::Struct || td.constructors.iter().any(|c| c.params.is_empty()))
                .unwrap_or(false),
            Ty::String
            | Ty::Array { .. }
            | Ty::Ptr(_)
            | Ty::Nullable(_)
            | Ty::Dyn
            | Ty::Error => true,
            Ty::Bool
            | Ty::Byte
            | Ty::SByte
            | Ty::Short
            | Ty::UShort
            | Ty::Int
            | Ty::UInt
            | Ty::Long
            | Ty::ULong
            | Ty::Float
            | Ty::Double
            | Ty::Decimal
            | Ty::Char => true,
            _ => false,
        }
    }

    fn check_inheritance(&mut self) {
        let names: Vec<_> = self.inheritance.keys().cloned().collect();
        for child in names {
            let bases = self.inheritance.get(&child).cloned().unwrap_or_default();
            for base in bases {
                if !self.types.contains_key(&base) {
                    let span = self.types.get(&child).map(|t| t.span).unwrap_or_default();
                    self.err(
                        format!("type '{}' inherits unknown type '{}'", child, base),
                        span,
                    );
                    continue;
                }
                // Cycle if `child` is already an ancestor of `base`
                // (i.e. base already extends child transitively).
                if child != base && self.is_ancestor(&child, &base) {
                    let span = self.types.get(&child).map(|t| t.span).unwrap_or_default();
                    self.err(
                        format!("inheritance cycle involving '{}'", child),
                        span,
                    );
                }
            }
        }

        // Check overrides
        let type_names: Vec<_> = self.types.keys().cloned().collect();
        for name in type_names {
            let bases = self.inheritance.get(&name).cloned().unwrap_or_default();
            if bases.is_empty() {
                continue;
            }
            for base in &bases {
                if let Some(base_td) = self.types.get(base).cloned() {
                    if base_td.kind == TypeDefKind::Interface {
                        self.check_interface_contract(
                            &name,
                            base,
                            &base_td.methods,
                            &base_td.properties,
                        );
                    }
                }
            }
            let methods: Vec<(String, FuncSig)> = self
                .types
                .get(&name)
                .map(|t| t.methods.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default();
            for (mname, sig) in methods {
                for base in &bases {
                    if let Some(base_sig) = self.lookup_method_in_type(base, &mname) {
                        if sig.params.len() != base_sig.params.len()
                            || sig.ret != base_sig.ret
                                && !self.is_semantically_assignable(&sig.ret, &base_sig.ret)
                        {
                            self.err(
                                format!(
                                    "method '{}.{}' does not match overridden '{}.{}'",
                                    name, mname, base, mname
                                ),
                                sig.span,
                            );
                        }
                    }
                }
            }
        }
    }

    fn check_interface_contract(
        &mut self,
        child: &str,
        interface_name: &str,
        iface_methods: &HashMap<String, FuncSig>,
        iface_properties: &HashMap<String, Ty>,
    ) {
        let Some(child_td) = self.types.get(child).cloned() else {
            return;
        };
        for (name, sig) in iface_methods {
            let Some(found) = child_td.methods.get(name) else {
                self.err(
                    format!("type '{}' does not implement interface method '{}.{}'", child, interface_name, name),
                    child_td.span,
                );
                continue;
            };
            if found.params.len() != sig.params.len()
                || !self.is_semantically_assignable(&found.ret, &sig.ret)
                || found.is_static != sig.is_static
            {
                self.err(
                    format!("method '{}.{}' does not match interface '{}.{}'", child, name, interface_name, name),
                    found.span,
                );
            }
        }
        for (name, ty) in iface_properties {
            let Some(found) = child_td.properties.get(name) else {
                self.err(
                    format!("type '{}' does not implement interface property '{}.{}'", child, interface_name, name),
                    child_td.span,
                );
                continue;
            };
            if !self.is_semantically_assignable(found, ty)
                || !self.is_semantically_assignable(ty, found)
            {
                self.err(
                    format!("property '{}.{}' does not match interface '{}.{}'", child, name, interface_name, name),
                    child_td.span,
                );
            }
        }
    }

    fn is_ancestor(&self, ancestor: &str, node: &str) -> bool {
        if ancestor == node {
            return true;
        }
        if let Some(bases) = self.inheritance.get(node) {
            for b in bases {
                if self.is_ancestor(ancestor, b) {
                    return true;
                }
            }
        }
        false
    }

    fn is_subtype(&self, child: &Ty, parent: &Ty) -> bool {
        match (child, parent) {
            (Ty::Named(c), Ty::Named(p)) => self.is_ancestor(p, c),
            (Ty::Generic { name: c, .. }, Ty::Named(p)) => self.is_ancestor(p, c),
            (Ty::Generic { name: c, .. }, Ty::Generic { name: p, .. }) => self.is_ancestor(p, c),
            _ => false,
        }
    }

    fn lookup_method_in_type(&self, type_name: &str, method: &str) -> Option<FuncSig> {
        let mut current = Some(type_name.to_string());
        while let Some(name) = current {
            if let Some(td) = self.types.get(&name) {
                if let Some(m) = td.methods.get(method) {
                    return Some(m.clone());
                }
                current = td.bases.first().cloned();
            } else {
                break;
            }
        }
        None
    }

    // ─── Check bodies ────────────────────────────────────────

    fn check_item(&mut self, item: &Item) {
        match item {
            Item::Attribute(_, inner) => self.check_item(inner),
            Item::Namespace(ns) => {
                for i in &ns.items {
                    self.check_item(i);
                }
            }
            Item::Class(c) => self.check_class(c),
            Item::Struct(s) => self.check_struct(s),
            Item::Union(u) => {
                self.check_struct(&StructDecl {
                    access: u.access.clone(),
                    name: u.name.clone(),
                    type_params: vec![],
                    members: u.members.clone(),
                    attributes: u.attributes.clone(),
                    packed: u.packed,
                    align: u.align,
                    repr_c: true,
                    span: u.span,
                });
            }
            Item::Interface(_) => {} // signatures only
            Item::Function(f) => {
                if f.is_static {
                    self.err(
                        format!(
                            "'static' is only valid on type members, not on top-level function '{}'",
                            f.name
                        ),
                        f.span,
                    );
                }
                self.check_function(f, None, None)
            },
            Item::Const(c) => {
                let expected = self.resolve_type_ref(&c.ty);
                let actual = self.check_expr(&c.value);
                self.expect_assignable(&actual, &expected, c.span, "const initializer");
            }
            Item::Import(_) | Item::Module(_) => {}
        }
    }

    fn check_class(&mut self, c: &ClassDecl) {
        let saved_tp = self.type_params.clone();
        for tp in &c.type_params {
            self.type_params.insert(tp.clone());
        }
        let this = if c.type_params.is_empty() {
            Ty::Named(c.name.clone())
        } else {
            Ty::Generic {
                name: c.name.clone(),
                args: c
                    .type_params
                    .iter()
                    .map(|t| Ty::TypeParam(t.clone()))
                    .collect(),
            }
        };
        let base = c.bases.first().map(|b| Ty::Named(b.name.clone()));

        for m in &c.members {
            match m {
                Member::Field(f) => {
                    if let Some(init) = &f.init {
                        let expected = f
                            .ty
                            .as_ref()
                            .map(|t| self.resolve_type_ref(t))
                            .unwrap_or(Ty::Dyn);
                        let actual = {
                            self.this_ty = Some(this.clone());
                            self.base_ty = base.clone();
                            self.check_expr(init)
                        };
                        self.expect_assignable(&actual, &expected, f.span, "field initializer");
                    }
                }
                Member::Method(f) => {
                    if f.is_static && (f.is_virtual || f.is_override) {
                        self.err(
                            format!("static method '{}' cannot be virtual or override", f.name),
                            f.span,
                        );
                    }
                    if let Some(base_ty) = &base {
                        let base_name = match base_ty {
                            Ty::Named(name) => Some(name.as_str()),
                            Ty::Generic { name, .. } => Some(name.as_str()),
                            _ => None,
                        };
                        if let Some(base_name) = base_name {
                            let base_has_method = self.lookup_method_in_type(base_name, &f.name).is_some();
                            let base_is_class = self
                                .types
                                .get(base_name)
                                .map(|td| td.kind == TypeDefKind::Class)
                                .unwrap_or(false);
                            if base_is_class && base_has_method && !f.is_override && !f.is_static {
                                self.err(
                                    format!(
                                        "method '{}.{}' hides a base member; mark it as override",
                                        c.name, f.name
                                    ),
                                    f.span,
                                );
                            }
                            if f.is_override && (!base_is_class || !base_has_method) {
                                self.err(
                                    format!(
                                        "method '{}.{}' is marked override but no base member exists",
                                        c.name, f.name
                                    ),
                                    f.span,
                                );
                            }
                        }
                    }
                    if f.is_abstract && f.body.is_some() {
                        self.err(
                            format!("abstract method '{}' cannot have a body", f.name),
                            f.span,
                        );
                    }
                    if !f.is_abstract && f.body.is_none() && !c.is_abstract {
                        // interface-like empty method on concrete class
                        self.warn(
                            format!("method '{}' has no body", f.name),
                            f.span,
                        );
                    }
                    let this_for_method = if f.is_static { None } else { Some(this.clone()) };
                    let base_for_method = if f.is_static { None } else { base.clone() };
                    self.check_function(f, this_for_method, base_for_method);
                }
                Member::Constructor(ctor) => {
                    self.check_constructor(ctor, this.clone(), base.clone());
                }
                Member::Destructor(d) => {
                    self.this_ty = Some(this.clone());
                    self.base_ty = base.clone();
                    self.expected_return = Some(Ty::Void);
                    self.push_scope();
                    self.check_block(&d.body);
                    self.pop_scope();
                    self.this_ty = None;
                    self.base_ty = None;
                    self.expected_return = None;
                }
                Member::Property(p) => {
                    let ty = self.resolve_type_ref(&p.ty);
                    self.this_ty = if p.is_static { None } else { Some(this.clone()) };
                    if let Some(g) = &p.getter {
                        self.expected_return = Some(ty.clone());
                        self.push_scope();
                        self.check_block(g);
                        self.pop_scope();
                    }
                    if let Some(s) = &p.setter {
                        self.expected_return = Some(Ty::Void);
                        self.push_scope();
                        self.declare_local("value", ty.clone(), false, p.span);
                        self.check_block(s);
                        self.pop_scope();
                    }
                    self.this_ty = None;
                    self.expected_return = None;
                }
                Member::Indexer(idx) => {
                    let ty = self.resolve_type_ref(&idx.ty);
                    self.this_ty = Some(this.clone());
                    self.push_scope();
                    for p in &idx.params {
                        let pt = self.resolve_type_ref(&p.ty);
                        self.declare_local(&p.name, pt, false, p.span);
                    }
                    if let Some(g) = &idx.getter {
                        self.expected_return = Some(ty.clone());
                        self.check_block(g);
                    }
                    if let Some(s) = &idx.setter {
                        self.expected_return = Some(Ty::Void);
                        self.declare_local("value", ty, false, idx.span);
                        self.check_block(s);
                    }
                    self.pop_scope();
                    self.this_ty = None;
                    self.expected_return = None;
                }
                Member::Operator(op) => {
                    self.expected_return = Some(self.resolve_type_ref(&op.return_type));
                    self.push_scope();
                    for p in &op.params {
                        let pt = self.resolve_type_ref(&p.ty);
                        self.declare_local(&p.name, pt, false, p.span);
                    }
                    self.check_block(&op.body);
                    self.pop_scope();
                    self.expected_return = None;
                }
            }
        }
        self.type_params = saved_tp;
    }

    fn check_struct(&mut self, s: &StructDecl) {
        let saved_tp = self.type_params.clone();
        for tp in &s.type_params {
            self.type_params.insert(tp.clone());
        }
        let this = Ty::Named(s.name.clone());
        for m in &s.members {
            match m {
                Member::Method(f) => {
                    let this_for_method = if f.is_static { None } else { Some(this.clone()) };
                    self.check_function(f, this_for_method, None)
                }
                Member::Constructor(ctor) => {
                    self.check_constructor(ctor, this.clone(), None);
                }
                Member::Field(f) => {
                    if let Some(init) = &f.init {
                        let expected = f
                            .ty
                            .as_ref()
                            .map(|t| self.resolve_type_ref(t))
                            .unwrap_or(Ty::Dyn);
                        let actual = self.check_expr(init);
                        self.expect_assignable(&actual, &expected, f.span, "field initializer");
                    }
                }
                _ => {}
            }
        }
        self.type_params = saved_tp;
    }

    fn check_constructor(&mut self, ctor: &ConstructorDecl, this: Ty, base: Option<Ty>) {
        self.this_ty = Some(this);
        self.base_ty = base;
        self.expected_return = Some(Ty::Void);
        self.push_scope();
        for p in &ctor.params {
            let ty = self.resolve_type_ref(&p.ty);
            self.declare_local(&p.name, ty, false, p.span);
            if let Some(def) = &p.default {
                let expected = self.resolve_type_ref(&p.ty);
                let dt = self.check_expr(def);
                self.expect_assignable(&dt, &expected, p.span, "default argument");
            }
        }
        for a in &ctor.base_args {
            let _ = self.check_expr(a);
        }
        self.check_block(&ctor.body);
        self.pop_scope();
        self.this_ty = None;
        self.base_ty = None;
        self.expected_return = None;
    }

    fn check_function(&mut self, f: &FunctionDecl, this: Option<Ty>, base: Option<Ty>) {
        // Bodyless = FFI import (or abstract); nothing to check in body.
        if f.body.is_none() {
            let saved_tp = self.type_params.clone();
            for tp in &f.type_params {
                self.type_params.insert(tp.clone());
            }
            let _ = self.resolve_type_ref(&f.return_type);
            for p in &f.params {
                let _ = self.resolve_type_ref(&p.ty);
            }
            self.type_params = saved_tp;
            return;
        }
        let saved_tp = self.type_params.clone();
        for tp in &f.type_params {
            self.type_params.insert(tp.clone());
        }
        let declared = self.resolve_type_ref(&f.return_type);
        // Inside an async function, `return` / expression body use the inner type.
        let body_ret = if f.is_async {
            unwrap_task_return(&declared)
        } else {
            declared
        };
        self.expected_return = Some(body_ret.clone());
        self.this_ty = this;
        self.base_ty = base;
        self.push_scope();

        for p in &f.params {
            let ty = self.resolve_type_ref(&p.ty);
            if p.is_this {
                // extension this — bind as local
            }
            self.declare_local(&p.name, ty.clone(), false, p.span);
            if let Some(def) = &p.default {
                let dt = self.check_expr(def);
                self.expect_assignable(&dt, &ty, p.span, "default argument");
            }
        }

        match &f.body {
            Some(FunctionBody::Block(b)) => self.check_block(b),
            Some(FunctionBody::Expr(e)) => {
                let actual = self.check_expr(e);
                if !matches!(body_ret, Ty::Void) {
                    self.expect_assignable(&actual, &body_ret, f.span, "function expression body");
                }
            }
            None => {}
        }

        self.pop_scope();
        self.expected_return = None;
        self.this_ty = None;
        self.base_ty = None;
        self.type_params = saved_tp;
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare_local(&mut self, name: &str, ty: Ty, is_const: bool, span: Span) {
        let duplicate = self
            .scopes
            .last()
            .map(|scope| scope.contains_key(name))
            .unwrap_or(false);
        if duplicate {
            self.err(
                format!("variable '{}' already declared in this scope", name),
                span,
            );
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name.to_string(),
                ScopeVar {
                    ty,
                    is_const,
                    span,
                },
            );
        } else {
            self.globals.insert(
                name.to_string(),
                ScopeVar {
                    ty,
                    is_const,
                    span,
                },
            );
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&ScopeVar> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        self.globals.get(name)
    }

    fn check_block(&mut self, block: &Block) {
        self.push_scope();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        self.pop_scope();
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Expr(e) => {
                let _ = self.check_expr(e);
            }
            Stmt::Decl(d) => self.check_var_decl(d),
            Stmt::Const(c) => {
                let expected = self.resolve_type_ref(&c.ty);
                let actual = self.check_expr(&c.value);
                self.expect_assignable(&actual, &expected, c.span, "const");
                self.declare_local(&c.name, expected, true, c.span);
            }
            Stmt::Return(expr, span) => {
                let expected = self.expected_return.clone().unwrap_or(Ty::Void);
                match (expr, &expected) {
                    (None, Ty::Void) => {}
                    (None, _) => {
                        self.err(
                            format!("function must return a value of type '{}'", expected),
                            *span,
                        );
                    }
                    (Some(e), Ty::Void) => {
                        let _ = self.check_expr(e);
                        self.err("void function cannot return a value", *span);
                    }
                    (Some(e), exp) => {
                        let actual = self.check_expr(e);
                        self.expect_assignable(&actual, exp, *span, "return");
                    }
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_branch,
                span,
            } => {
                let ct = self.check_expr(cond);
                if !ct.is_bool_like() {
                    self.err(
                        format!("if condition must be bool, found '{}'", ct),
                        *span,
                    );
                }
                self.check_block(then_block);
                if let Some(e) = else_branch {
                    match e {
                        ElseBranch::Block(b) => self.check_block(b),
                        ElseBranch::If(s) => self.check_stmt(s),
                    }
                }
            }
            Stmt::While { cond, body, span } => {
                let ct = self.check_expr(cond);
                if !ct.is_bool_like() {
                    self.err(
                        format!("while condition must be bool, found '{}'", ct),
                        *span,
                    );
                }
                self.in_loop += 1;
                self.check_block(body);
                self.in_loop -= 1;
            }
            Stmt::DoWhile { body, cond, span } => {
                self.in_loop += 1;
                self.check_block(body);
                self.in_loop -= 1;
                let ct = self.check_expr(cond);
                if !ct.is_bool_like() {
                    self.err(
                        format!("do-while condition must be bool, found '{}'", ct),
                        *span,
                    );
                }
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
                ..
            } => {
                self.push_scope();
                if let Some(i) = init {
                    self.check_stmt(i);
                }
                if let Some(c) = cond {
                    let ct = self.check_expr(c);
                    if !ct.is_bool_like() {
                        self.err(
                            format!("for condition must be bool, found '{}'", ct),
                            c.span(),
                        );
                    }
                }
                if let Some(s) = step {
                    let _ = self.check_expr(s);
                }
                self.in_loop += 1;
                self.check_block(body);
                self.in_loop -= 1;
                self.pop_scope();
            }
            Stmt::Foreach {
                var_name,
                index_name,
                iter,
                body,
                span,
            } => {
                let it = self.check_expr(iter);
                let elem = self.element_type(&it).unwrap_or_else(|| {
                    self.err(
                        format!("foreach requires an iterable type, found '{}'", it),
                        *span,
                    );
                    Ty::Dyn
                });
                self.push_scope();
                self.declare_local(var_name, elem, false, *span);
                if let Some(idx) = index_name {
                    self.declare_local(idx, Ty::Int, false, *span);
                }
                self.in_loop += 1;
                self.check_block(body);
                self.in_loop -= 1;
                self.pop_scope();
            }
            Stmt::Switch { expr, cases, .. } => {
                let st = self.check_expr(expr);
                for case in cases {
                    for pat in &case.patterns {
                        let st_for_case = st.clone();
                        let check_pat = |sema: &mut Self, e: &Expr| {
                            let pt = sema.check_expr(e);
                            if !sema.is_semantically_assignable(&pt, &st_for_case)
                                && !sema.is_semantically_assignable(&st_for_case, &pt)
                                && !matches!(st_for_case, Ty::Dyn)
                                && !matches!(pt, Ty::Dyn)
                            {
                                sema.warn(
                                    format!(
                                        "switch case type '{}' may not match switch expression '{}'",
                                        pt, st_for_case
                                    ),
                                    e.span(),
                                );
                            }
                        };
                        match pat {
                            crate::ast::SwitchPattern::Expr(e) => check_pat(self, e),
                            crate::ast::SwitchPattern::Range(lo, hi) => {
                                check_pat(self, lo);
                                check_pat(self, hi);
                            }
                        }
                    }
                    if let Some(g) = &case.guard {
                        self.check_expr(g);
                    }
                    self.in_loop += 1; // allow break
                    for s in &case.body {
                        self.check_stmt(s);
                    }
                    self.in_loop -= 1;
                }
            }
            Stmt::Match { expr, arms, .. } => {
                let _ = self.check_expr(expr);
                for arm in arms {
                    self.push_scope();
                    if let Some(bind) = &arm.bind {
                        self.declare_local(bind, Ty::Dyn, false, arm.body.span());
                    }
                    let _ = self.check_expr(&arm.body);
                    self.pop_scope();
                }
            }
            Stmt::Try {
                body,
                catches,
                finally,
                ..
            } => {
                self.check_block(body);
                for c in catches {
                    self.push_scope();
                    if let Some(name) = &c.name {
                        let ty = c
                            .exception_type
                            .as_ref()
                            .map(|t| self.resolve_type_ref(t))
                            .unwrap_or(Ty::Named("Exception".into()));
                        self.declare_local(name, ty, false, c.body.span);
                    }
                    self.check_block(&c.body);
                    self.pop_scope();
                }
                if let Some(f) = finally {
                    self.check_block(f);
                }
            }
            Stmt::Throw(e, _) => {
                let _ = self.check_expr(e);
            }
            Stmt::Break(span) => {
                if self.in_loop == 0 {
                    self.err("'break' outside of loop or switch", *span);
                }
            }
            Stmt::Continue(span) => {
                if self.in_loop == 0 {
                    self.err("'continue' outside of loop", *span);
                }
            }
            Stmt::Using { decl, body, .. } => {
                self.push_scope();
                self.check_var_decl(decl);
                self.check_block(body);
                self.pop_scope();
            }
            Stmt::Unsafe(body, _) => {
                let prev = self.in_unsafe;
                self.in_unsafe = true;
                self.check_block(body);
                self.in_unsafe = prev;
            }
            Stmt::Asm { span, .. } => {
                if !self.in_unsafe {
                    self.err("inline asm requires an 'unsafe' block", *span);
                }
            }
            Stmt::Block(b) => self.check_block(b),
        }
    }

    fn check_var_decl(&mut self, d: &VarDecl) {
        let ty = match d.kind {
            VarKind::Dyn => Ty::Dyn,
            VarKind::Var | VarKind::Stack | VarKind::Owned => {
                if let Some(tr) = &d.ty {
                    self.resolve_type_ref(tr)
                } else if let Some(init) = &d.init {
                    let t = self.check_expr(init);
                    if matches!(t, Ty::Null | Ty::Void) {
                        self.err(
                            format!(
                                "cannot infer type for '{}' from '{}'; provide an explicit type",
                                d.name, t
                            ),
                            d.span,
                        );
                        Ty::Dyn
                    } else {
                        t
                    }
                } else {
                    self.err(
                        format!("variable '{}' needs a type or initializer", d.name),
                        d.span,
                    );
                    Ty::Dyn
                }
            }
            VarKind::Typed | VarKind::Const => {
                let declared = d
                    .ty
                    .as_ref()
                    .map(|t| self.resolve_type_ref(t))
                    .unwrap_or(Ty::Dyn);
                if let Some(init) = &d.init {
                    let actual = self.check_expr(init);
                    self.expect_assignable(&actual, &declared, d.span, "variable initializer");
                }
                declared
            }
        };

        // For var with init already checked above for inference path —
        // for Dyn still check init
        if matches!(d.kind, VarKind::Dyn) {
            if let Some(init) = &d.init {
                let _ = self.check_expr(init);
            }
        } else if matches!(d.kind, VarKind::Var | VarKind::Stack | VarKind::Owned) && d.ty.is_some() {
            if let Some(init) = &d.init {
                let actual = self.check_expr(init);
                self.expect_assignable(&actual, &ty, d.span, "variable initializer");
            }
        }

        let is_const = matches!(d.kind, VarKind::Const);
        self.declare_local(&d.name, ty, is_const, d.span);
    }

    fn element_type(&self, ty: &Ty) -> Option<Ty> {
        match ty {
            Ty::Array { elem, .. } => Some((**elem).clone()),
            Ty::Generic { name, args } if name == "List" || name == "Set" || name == "Queue" || name == "Stack" => {
                args.first().cloned()
            }
            Ty::String => Some(Ty::Char),
            Ty::Dyn => Some(Ty::Dyn),
            _ => None,
        }
    }

    // ─── Expressions ─────────────────────────────────────────

    fn check_expr(&mut self, expr: &Expr) -> Ty {
        match expr {
            Expr::Int(_, _) => Ty::Int,
            Expr::UInt(_, _) => Ty::UInt,
            Expr::Float(_, _) => Ty::Double,
            Expr::Decimal(_, _) => Ty::Decimal,
            Expr::Bool(_, _) => Ty::Bool,
            Expr::Char(_, _) => Ty::Char,
            Expr::String(_, _) => Ty::String,
            Expr::Null(_) => Ty::Null,
            Expr::Interpolated(parts, _) => {
                for p in parts {
                    if let InterpPart::Expr(e) = p {
                        let _ = self.check_expr(e);
                    }
                }
                Ty::String
            }
            Expr::Ident(name, span) => self.check_ident(name, *span),
            Expr::This(span) => self
                .this_ty
                .clone()
                .unwrap_or_else(|| {
                    self.err("'this' is not valid outside an instance method", *span);
                    Ty::Error
                }),
            Expr::Base(span) => self
                .base_ty
                .clone()
                .unwrap_or_else(|| {
                    self.err("'base' is not valid here", *span);
                    Ty::Error
                }),
            Expr::Grouped(e, _) => self.check_expr(e),
            Expr::Binary {
                left, op, right, span, ..
            } => self.check_binary(left, *op, right, *span),
            Expr::Unary { op, expr, span } => self.check_unary(*op, expr, *span),
            Expr::Assign {
                target,
                op,
                value,
                span,
            } => self.check_assign(target, *op, value, *span),
            Expr::Call {
                callee,
                type_args,
                args,
                span,
            } => self.check_call(callee, type_args, args, *span),
            Expr::Member {
                object,
                field,
                null_safe,
                span,
            } => self.check_member(object, field, *null_safe, *span),
            Expr::Index {
                object,
                indices,
                span,
            } => self.check_index(object, indices, *span),
            Expr::New {
                ty,
                args,
                init,
                span,
            } => self.check_new(ty, args, init, *span),
            Expr::ArrayLit(elems, span) => self.check_array_lit(elems, *span),
            Expr::Lambda { params, body, span } => self.check_lambda(params, body, *span),
            Expr::Ternary {
                cond,
                then_expr,
                else_expr,
                span,
            } => {
                let ct = self.check_expr(cond);
                if !ct.is_bool_like() {
                    self.err(
                        format!("ternary condition must be bool, found '{}'", ct),
                        *span,
                    );
                }
                let a = self.check_expr(then_expr);
                let b = self.check_expr(else_expr);
                Ty::unify(&a, &b)
            }
            Expr::Cast { ty, expr, .. } => {
                let from = self.check_expr(expr);
                let to = self.resolve_type_ref(ty);
                // Allow most casts; warn on obviously impossible
                if !self.is_semantically_assignable(&from, &to)
                    && !self.is_semantically_assignable(&to, &from)
                    && !from.is_numeric()
                    && !to.is_numeric()
                    && !matches!(from, Ty::Dyn | Ty::Error)
                    && !matches!(to, Ty::Dyn | Ty::Error)
                {
                    self.warn(
                        format!("cast from '{}' to '{}' may be invalid", from, to),
                        ty.span,
                    );
                }
                to
            }
            Expr::TypeOf(_, _) => Ty::Named("Type".into()),
            Expr::SizeOf(ty, _) => {
                let _ = self.resolve_type_ref(ty);
                Ty::Int
            }
            Expr::OffsetOf { ty, .. } => {
                let _ = self.resolve_type_ref(ty);
                Ty::Int
            }
            Expr::Is { expr, .. } => {
                let _ = self.check_expr(expr);
                Ty::Bool
            }
            Expr::As { expr, ty, .. } => {
                let _ = self.check_expr(expr);
                self.resolve_type_ref(ty).make_nullable()
            }
            Expr::Await(e, _) => {
                let t = self.check_expr(e);
                // Task<T> → T, else dyn
                match t {
                    Ty::Generic { name, args } if name == "Task" => {
                        args.into_iter().next().unwrap_or(Ty::Void)
                    }
                    other => other,
                }
            }
            Expr::Deref(e, span) => {
                if !self.in_unsafe {
                    self.err("pointer dereference requires 'unsafe' block", *span);
                }
                match self.check_expr(e) {
                    Ty::Ptr(inner) => *inner,
                    Ty::Dyn => Ty::Dyn,
                    other => {
                        self.err(
                            format!("cannot dereference type '{}'", other),
                            *span,
                        );
                        Ty::Error
                    }
                }
            }
            Expr::AddressOf(e, span) => {
                if !self.in_unsafe {
                    self.err("address-of requires 'unsafe' block", *span);
                }
                Ty::Ptr(Box::new(self.check_expr(e)))
            }
            Expr::PtrMember { object, field, span } => {
                if !self.in_unsafe {
                    self.err("pointer member access requires 'unsafe' block", *span);
                }
                let ot = match self.check_expr(object) {
                    Ty::Ptr(inner) => *inner,
                    other => other,
                };
                self.lookup_field_or_prop(&ot, field, *span)
            }
            Expr::Try(e, span) => {
                let t = self.check_expr(e);
                match t {
                    Ty::Generic { name, args } if name == "Result" => {
                        args.first().cloned().unwrap_or(Ty::Dyn)
                    }
                    Ty::Nullable(inner) => {
                        // ? on nullable propagates
                        *inner
                    }
                    other => {
                        self.warn(
                            format!("'?' on type '{}' may not be a Result", other),
                            *span,
                        );
                        other
                    }
                }
            }
        }
    }

    fn check_ident(&mut self, name: &str, span: Span) -> Ty {
        if let Some(v) = self.lookup_var(name) {
            return v.ty.clone();
        }
        if let Some(f) = self.functions.get(name) {
            return Ty::Func {
                params: f.params.iter().map(|(_, t)| t.clone()).collect(),
                ret: Box::new(f.ret.clone()),
            };
        }
        if self.types.contains_key(name) {
            // type used as value (e.g. int.Parse) — return a type-object
            return Ty::Named(name.to_string());
        }
        // Built-in type names as expressions
        if is_primitive_name(name) {
            return Ty::Named(name.to_string());
        }
        self.resolve_msg(format!("undefined name '{}'", name), span);
        Ty::Error
    }

    fn check_binary(&mut self, left: &Expr, op: BinOp, right: &Expr, span: Span) -> Ty {
        let lt = self.check_expr(left);
        let rt = self.check_expr(right);

        if matches!(lt, Ty::Dyn) || matches!(rt, Ty::Dyn) || matches!(lt, Ty::Error) || matches!(rt, Ty::Error)
        {
            return match op {
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::And | BinOp::Or => {
                    Ty::Bool
                }
                _ => Ty::Dyn,
            };
        }

        match op {
            BinOp::Add => {
                if matches!(lt, Ty::String) || matches!(rt, Ty::String) {
                    return Ty::String;
                }
                if let Some(t) = Ty::promote_numeric(&lt, &rt) {
                    return t;
                }
                // Allow overloaded operators on named types
                if matches!((&lt, &rt), (Ty::Named(a), Ty::Named(b)) if a == b) {
                    return lt;
                }
                self.err(
                    format!("operator '+' cannot be applied to '{}' and '{}'", lt, rt),
                    span,
                );
                Ty::Error
            }
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                if let Some(t) = Ty::promote_numeric(&lt, &rt) {
                    return t;
                }
                if matches!((&lt, &rt), (Ty::Named(a), Ty::Named(b)) if a == b) {
                    return lt;
                }
                self.err(
                    format!("operator '{:?}' cannot be applied to '{}' and '{}'", op, lt, rt),
                    span,
                );
                Ty::Error
            }
            BinOp::Eq | BinOp::Ne => {
                if self.is_semantically_assignable(&lt, &rt)
                    || self.is_semantically_assignable(&rt, &lt)
                    || matches!(lt, Ty::Null)
                    || matches!(rt, Ty::Null)
                    || self.is_subtype(&lt, &rt)
                    || self.is_subtype(&rt, &lt)
                {
                    Ty::Bool
                } else {
                    self.err(
                        format!("cannot compare '{}' and '{}' with '{:?}'", lt, rt, op),
                        span,
                    );
                    Ty::Bool
                }
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                if lt.is_numeric() && rt.is_numeric() || matches!(lt, Ty::Char) && matches!(rt, Ty::Char)
                {
                    Ty::Bool
                } else {
                    self.err(
                        format!("cannot compare '{}' and '{}' with relational operator", lt, rt),
                        span,
                    );
                    Ty::Bool
                }
            }
            BinOp::And | BinOp::Or => {
                if !lt.is_bool_like() || !rt.is_bool_like() {
                    self.err(
                        format!("logical operator requires bool operands, found '{}' and '{}'", lt, rt),
                        span,
                    );
                }
                Ty::Bool
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                if lt.is_integral() && rt.is_integral() {
                    Ty::promote_numeric(&lt, &rt).unwrap_or(Ty::Int)
                } else {
                    self.err(
                        format!("bitwise operator requires integral types, found '{}' and '{}'", lt, rt),
                        span,
                    );
                    Ty::Error
                }
            }
            BinOp::NullCoalesce => {
                let left_inner = lt.unwrap_nullable().clone();
                Ty::unify(&left_inner, &rt)
            }
        }
    }

    fn check_unary(&mut self, op: UnOp, expr: &Expr, span: Span) -> Ty {
        let t = self.check_expr(expr);
        match op {
            UnOp::Neg => {
                if t.is_numeric() {
                    t
                } else {
                    self.err(format!("unary '-' cannot be applied to '{}'", t), span);
                    Ty::Error
                }
            }
            UnOp::Not => {
                if t.is_bool_like() {
                    Ty::Bool
                } else {
                    self.err(format!("unary '!' cannot be applied to '{}'", t), span);
                    Ty::Bool
                }
            }
            UnOp::BitNot => {
                if t.is_integral() {
                    t
                } else {
                    self.err(format!("unary '~' cannot be applied to '{}'", t), span);
                    Ty::Error
                }
            }
            UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec => {
                if !self.is_lvalue(expr) {
                    self.err("increment/decrement requires an assignable variable", span);
                }
                if t.is_numeric() || matches!(t, Ty::Char) {
                    t
                } else {
                    self.err(
                        format!("cannot apply increment/decrement to '{}'", t),
                        span,
                    );
                    Ty::Error
                }
            }
        }
    }

    fn is_lvalue(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Ident(name, _) => {
                if let Some(v) = self.lookup_var(name) {
                    !v.is_const
                } else {
                    true
                }
            }
            Expr::Member { .. } | Expr::Index { .. } | Expr::PtrMember { .. } | Expr::Deref(_, _) => {
                true
            }
            Expr::Grouped(e, _) => self.is_lvalue(e),
            _ => false,
        }
    }

    fn check_assign(&mut self, target: &Expr, op: AssignOp, value: &Expr, span: Span) -> Ty {
        if !self.is_lvalue(target) {
            self.err("left-hand side of assignment is not assignable", span);
        }
        if let Expr::Ident(name, _) = target {
            if let Some(v) = self.lookup_var(name) {
                if v.is_const {
                    self.err(format!("cannot assign to const '{}'", name), span);
                }
            }
        }
        let tt = self.check_expr(target);
        let vt = self.check_expr(value);

        match op {
            AssignOp::Assign => {
                self.expect_assignable(&vt, &tt, span, "assignment");
                vt
            }
            AssignOp::NullCoalesce => {
                let inner = tt.unwrap_nullable().clone();
                self.expect_assignable(&vt, &inner, span, "??=");
                vt
            }
            _ => {
                // compound: target = target op value
                let bin = match op {
                    AssignOp::Add => BinOp::Add,
                    AssignOp::Sub => BinOp::Sub,
                    AssignOp::Mul => BinOp::Mul,
                    AssignOp::Div => BinOp::Div,
                    AssignOp::Mod => BinOp::Mod,
                    AssignOp::BitAnd => BinOp::BitAnd,
                    AssignOp::BitOr => BinOp::BitOr,
                    AssignOp::BitXor => BinOp::BitXor,
                    AssignOp::Shl => BinOp::Shl,
                    AssignOp::Shr => BinOp::Shr,
                    _ => unreachable!(),
                };
                // Reuse binary rules without double-checking exprs — synthesize
                let result = self.binary_result_ty(&tt, bin, &vt, span);
                self.expect_assignable(&result, &tt, span, "compound assignment");
                result
            }
        }
    }

    fn binary_result_ty(&mut self, lt: &Ty, op: BinOp, rt: &Ty, span: Span) -> Ty {
        // Simplified duplicate of check_binary without re-checking
        if matches!(lt, Ty::Dyn) || matches!(rt, Ty::Dyn) {
            return Ty::Dyn;
        }
        match op {
            BinOp::Add if matches!(lt, Ty::String) || matches!(rt, Ty::String) => Ty::String,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                Ty::promote_numeric(lt, rt).unwrap_or_else(|| {
                    self.err(
                        format!("invalid compound assignment with '{}' and '{}'", lt, rt),
                        span,
                    );
                    Ty::Error
                })
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                if lt.is_integral() && rt.is_integral() {
                    Ty::promote_numeric(lt, rt).unwrap_or(Ty::Int)
                } else {
                    self.err("bitwise compound assignment requires integrals", span);
                    Ty::Error
                }
            }
            _ => Ty::Dyn,
        }
    }

    fn check_call(
        &mut self,
        callee: &Expr,
        type_args: &[TypeRef],
        args: &[Arg],
        span: Span,
    ) -> Ty {
        let arg_tys: Vec<Ty> = args.iter().map(|a| self.check_expr(&a.value)).collect();
        let type_args_ty: Vec<Ty> = type_args.iter().map(|t| self.resolve_type_ref(t)).collect();

        // Method call: obj.method(args)
        if let Expr::Member {
            object,
            field,
            null_safe,
            span: mspan,
        } = callee
        {
            let obj_ty = self.check_expr(object);
            if *null_safe && matches!(obj_ty, Ty::Null) {
                return Ty::Null;
            }
            let obj_ty = obj_ty.unwrap_nullable().clone();
            if let Some(mut sig) = self.find_method(&obj_ty, field) {
                self.instantiate_sig(&mut sig, &obj_ty, &type_args_ty);
                self.check_args(&sig, &arg_tys, span);
                return sig.ret;
            }
            // Property that is a function?
            let ft = self.lookup_field_or_prop(&obj_ty, field, *mspan);
            if let Ty::Func { params, ret } = ft {
                if params.len() != arg_tys.len() {
                    self.err(
                        format!(
                            "expected {} argument(s), found {}",
                            params.len(),
                            arg_tys.len()
                        ),
                        span,
                    );
                }
                return *ret;
            }
            if !matches!(obj_ty, Ty::Dyn | Ty::Error) {
                self.err(
                    format!("type '{}' has no method '{}'", obj_ty, field),
                    span,
                );
            }
            return Ty::Dyn;
        }

        // Static: Type.Method(args) where callee is already Member handled above.
        // Free function / local function value
        let callee_ty = self.check_expr(callee);

        // Direct function name
        if let Expr::Ident(name, _) = callee {
            if let Some(mut sig) = self.functions.get(name).cloned() {
                if !type_args_ty.is_empty() {
                    self.apply_type_args_to_sig(&mut sig, &type_args_ty);
                }
                self.check_args(&sig, &arg_tys, span);
                return sig.ret;
            }
            // Type name used with call? uncommon
            if let Some(td) = self.types.get(name) {
                // Treat as constructor if callable — prefer new
                if let Some(ctor) = td.constructors.first() {
                    let sig = ctor.clone();
                    self.check_args(&sig, &arg_tys, span);
                    return Ty::Named(name.clone());
                }
            }
        }

        match callee_ty {
            Ty::Func { params, ret } => {
                if params.len() != arg_tys.len() && !params.is_empty() {
                    // allow dyn print-style already handled
                    self.err(
                        format!(
                            "expected {} argument(s), found {}",
                            params.len(),
                            arg_tys.len()
                        ),
                        span,
                    );
                }
                for (i, (p, a)) in params.iter().zip(arg_tys.iter()).enumerate() {
                    if !self.is_semantically_assignable(a, p) {
                        self.err(
                            format!(
                                "argument {}: expected '{}', found '{}'",
                                i + 1,
                                p,
                                a
                            ),
                            span,
                        );
                    }
                }
                *ret
            }
            Ty::Dyn | Ty::Error => Ty::Dyn,
            other => {
                self.err(format!("type '{}' is not callable", other), span);
                Ty::Error
            }
        }
    }

    fn check_args(&mut self, sig: &FuncSig, arg_tys: &[Ty], span: Span) {
        // Special-case print/write: accept any arity
        if matches!(sig.name.as_str(), "print" | "write") {
            return;
        }
        let expected = sig.params.len();
        if arg_tys.len() > expected {
            self.err(
                format!(
                    "function '{}' expects {} argument(s), found {}",
                    sig.name, expected, arg_tys.len()
                ),
                span,
            );
            return;
        }
        // Allow fewer args when remaining params are dyn (optional predicates etc.)
        if arg_tys.len() < expected {
            let missing_ok = sig.params[arg_tys.len()..]
                .iter()
                .all(|(_, t)| matches!(t, Ty::Dyn));
            if !missing_ok {
                self.err(
                    format!(
                        "function '{}' expects {} argument(s), found {}",
                        sig.name, expected, arg_tys.len()
                    ),
                    span,
                );
                return;
            }
        }
        for (i, ((_, pt), at)) in sig.params.iter().zip(arg_tys.iter()).enumerate() {
            if matches!(pt, Ty::Dyn) {
                continue;
            }
            if !self.is_semantically_assignable(at, pt) {
                self.err(
                    format!(
                        "argument {} of '{}': expected '{}', found '{}'",
                        i + 1,
                        sig.name,
                        pt,
                        at
                    ),
                    span,
                );
            }
        }
    }

    fn find_method(&self, obj_ty: &Ty, name: &str) -> Option<FuncSig> {
        let type_name = match obj_ty {
            Ty::Named(n) => n.clone(),
            Ty::Generic { name, .. } => name.clone(),
            Ty::String => "string".into(),
            Ty::Array { elem, dims } => {
                let same = Ty::Array {
                    elem: elem.clone(),
                    dims: *dims,
                };
                let sig = match name {
                    "Distinct" | "Sort" | "SortAsc" | "SortDesc" | "Reverse" | "Copy" => {
                        FuncSig {
                            name: name.into(),
                            params: vec![],
                            ret: same,
                            type_params: vec![],
                            is_method: true,
                            is_static: false,
                            span: Span::default(),
                        }
                    }
                    "Take" | "Skip" | "Chunk" => FuncSig {
                        name: name.into(),
                        params: vec![("count".into(), Ty::Int)],
                        ret: if name == "Chunk" {
                            Ty::Array {
                                elem: Box::new(Ty::Array {
                                    elem: elem.clone(),
                                    dims: 1,
                                }),
                                dims: 1,
                            }
                        } else {
                            same
                        },
                        type_params: vec![],
                        is_method: true,
                        is_static: false,
                        span: Span::default(),
                    },
                    "Count" => FuncSig {
                        name: name.into(),
                        params: vec![],
                        ret: Ty::Int,
                        type_params: vec![],
                        is_method: true,
                        is_static: false,
                        span: Span::default(),
                    },
                    "IndexOf" => FuncSig {
                        name: name.into(),
                        params: vec![("item".into(), Ty::Dyn)],
                        ret: Ty::Int,
                        type_params: vec![],
                        is_method: true,
                        is_static: false,
                        span: Span::default(),
                    },
                    "Flatten" => FuncSig {
                        name: name.into(),
                        params: vec![],
                        ret: match &**elem {
                            Ty::Array { elem: inner, .. } => Ty::Array {
                                elem: inner.clone(),
                                dims: 1,
                            },
                            _ => same,
                        },
                        type_params: vec![],
                        is_method: true,
                        is_static: false,
                        span: Span::default(),
                    },
                    "Zip" => FuncSig {
                        name: name.into(),
                        params: vec![(
                            "other".into(),
                            Ty::Array {
                                elem: Box::new(Ty::Dyn),
                                dims: 1,
                            },
                        )],
                        ret: Ty::Array {
                            elem: Box::new(Ty::Array {
                                elem: Box::new(Ty::Dyn),
                                dims: 1,
                            }),
                            dims: 1,
                        },
                        type_params: vec![],
                        is_method: true,
                        is_static: false,
                        span: Span::default(),
                    },
                    _ => return None,
                };
                return Some(sig);
            }
            Ty::Dyn => return Some(FuncSig {
                name: name.into(),
                params: vec![],
                ret: Ty::Dyn,
                type_params: vec![],
                is_method: true,
                is_static: false,
                span: Span::default(),
            }),
            _ => return None,
        };
        self.lookup_method_in_type(&type_name, name)
            .filter(|sig| !sig.is_static)
    }

    fn instantiate_sig(&self, sig: &mut FuncSig, obj_ty: &Ty, type_args: &[Ty]) {
        let subst = self.build_subst(obj_ty, &sig.type_params, type_args);
        for (_, p) in sig.params.iter_mut() {
            *p = substitute(p, &subst);
        }
        sig.ret = substitute(&sig.ret, &subst);
    }

    fn apply_type_args_to_sig(&self, sig: &mut FuncSig, type_args: &[Ty]) {
        let mut subst = HashMap::new();
        for (i, tp) in sig.type_params.iter().enumerate() {
            if let Some(a) = type_args.get(i) {
                subst.insert(tp.clone(), a.clone());
            }
        }
        for (_, p) in sig.params.iter_mut() {
            *p = substitute(p, &subst);
        }
        sig.ret = substitute(&sig.ret, &subst);
    }

    fn build_subst(
        &self,
        obj_ty: &Ty,
        _sig_tps: &[String],
        type_args: &[Ty],
    ) -> HashMap<String, Ty> {
        let mut subst = HashMap::new();
        if let Ty::Generic { name, args } = obj_ty {
            if let Some(td) = self.types.get(name) {
                for (tp, a) in td.type_params.iter().zip(args.iter()) {
                    subst.insert(tp.clone(), a.clone());
                }
            }
        }
        // Explicit type args on call override
        // (handled by caller for free functions)
        let _ = type_args;
        subst
    }

    fn check_member(&mut self, object: &Expr, field: &str, null_safe: bool, span: Span) -> Ty {
        let ot = self.check_expr(object);
        if null_safe {
            let inner = ot.unwrap_nullable().clone();
            return self.lookup_field_or_prop(&inner, field, span).make_nullable();
        }
        // Static member: Type.Parse
        if let Expr::Ident(name, _) = object {
            if self.types.contains_key(name) || is_primitive_name(name) {
                if let Some(ty) = self.lookup_static_field_or_prop(name, field) {
                    return ty;
                }
                if let Some(sig) = self.lookup_method_in_type(name, field) {
                    if sig.is_static {
                        return Ty::Func {
                            params: sig.params.iter().map(|(_, t)| t.clone()).collect(),
                            ret: Box::new(sig.ret),
                        };
                    }
                }
            }
        }
        self.lookup_field_or_prop(&ot, field, span)
    }

    fn lookup_static_field_or_prop(&self, type_name: &str, field: &str) -> Option<Ty> {
        let mut current = Some(type_name.to_string());
        while let Some(name) = current {
            if let Some(td) = self.types.get(&name) {
                if let Some(t) = td.static_fields.get(field) {
                    return Some(t.clone());
                }
                if let Some(t) = td.static_properties.get(field) {
                    return Some(t.clone());
                }
                current = td.bases.first().cloned();
            } else {
                break;
            }
        }
        None
    }

    fn lookup_field_or_prop(&mut self, ot: &Ty, field: &str, span: Span) -> Ty {
        let ot = ot.unwrap_nullable();
        match ot {
            Ty::Dyn | Ty::Error => Ty::Dyn,
            Ty::String => {
                if let Some(td) = self.types.get("string") {
                    if let Some(t) = td.properties.get(field) {
                        return t.clone();
                    }
                    if let Some(m) = td.methods.get(field) {
                        return Ty::Func {
                            params: m.params.iter().map(|(_, t)| t.clone()).collect(),
                            ret: Box::new(m.ret.clone()),
                        };
                    }
                }
                if let Some(ext) = self.lookup_extension("string", field) {
                    return ext;
                }
                self.err(format!("string has no member '{}'", field), span);
                Ty::Error
            }
            Ty::Array { elem, dims } => {
                if field == "Length" || field == "Count" {
                    return Ty::Int;
                }
                let same = Ty::Array {
                    elem: elem.clone(),
                    dims: *dims,
                };
                let listy = Ty::Array {
                    elem: elem.clone(),
                    dims: 1,
                };
                return match field {
                    "Distinct" | "Sort" | "SortAsc" | "SortDesc" | "Reverse" | "Take"
                    | "Skip" | "Copy" => same,
                    "IndexOf" => Ty::Int,
                    "Chunk" => Ty::Array {
                        elem: Box::new(listy),
                        dims: 1,
                    },
                    "Flatten" => match &**elem {
                        Ty::Array { elem: inner, .. } => Ty::Array {
                            elem: inner.clone(),
                            dims: 1,
                        },
                        _ => same,
                    },
                    "Zip" => Ty::Array {
                        elem: Box::new(Ty::Array {
                            elem: Box::new(Ty::Dyn),
                            dims: 1,
                        }),
                        dims: 1,
                    },
                    _ => {
                        self.err(format!("array has no member '{}'", field), span);
                        Ty::Error
                    }
                };
            }
            Ty::Named(name) | Ty::Generic { name, .. } => {
                let mut current = Some(name.clone());
                while let Some(n) = current {
                    if let Some(td) = self.types.get(&n) {
                        if let Some(t) = td.fields.get(field) {
                            return substitute_for_obj(t, ot);
                        }
                        if let Some(t) = td.properties.get(field) {
                            return substitute_for_obj(t, ot);
                        }
                        if let Some(m) = td.methods.get(field) {
                            let mut sig = m.clone();
                            let subst = match ot {
                                Ty::Generic { args, .. } => {
                                    let mut s = HashMap::new();
                                    for (tp, a) in td.type_params.iter().zip(args.iter()) {
                                        s.insert(tp.clone(), a.clone());
                                    }
                                    s
                                }
                                _ => HashMap::new(),
                            };
                            for (_, p) in sig.params.iter_mut() {
                                *p = substitute(p, &subst);
                            }
                            sig.ret = substitute(&sig.ret, &subst);
                            return Ty::Func {
                                params: sig.params.iter().map(|(_, t)| t.clone()).collect(),
                                ret: Box::new(sig.ret),
                            };
                        }
                        current = td.bases.first().cloned();
                    } else {
                        break;
                    }
                }
                if let Some(ext) = self.lookup_extension(name, field) {
                    return ext;
                }
                self.err(format!("type '{}' has no member '{}'", ot, field), span);
                Ty::Error
            }
            other => {
                if let Ty::Named(n) = other {
                    if let Some(ext) = self.lookup_extension(n, field) {
                        return ext;
                    }
                }
                self.err(
                    format!("type '{}' has no member '{}'", other, field),
                    span,
                );
                Ty::Error
            }
        }
    }

    /// Extension method type for member access (drops the `this` parameter).
    fn lookup_extension(&self, type_name: &str, field: &str) -> Option<Ty> {
        let m = self.extensions.get(type_name)?.get(field)?;
        // Instance call site sees params after `this`
        let params: Vec<_> = m
            .params
            .iter()
            .skip(1)
            .map(|(_, t)| t.clone())
            .collect();
        Some(Ty::Func {
            params,
            ret: Box::new(m.ret.clone()),
        })
    }

    fn check_index(&mut self, object: &Expr, indices: &[Expr], span: Span) -> Ty {
        let ot = self.check_expr(object);
        // Class indexer get_Item
        if let Ty::Named(name) | Ty::Generic { name, .. } = ot.unwrap_nullable() {
            let indexer = self
                .types
                .get(name)
                .and_then(|td| td.methods.get("get_Item"))
                .cloned();
            if let Some(m) = indexer {
                for (i, idx) in indices.iter().enumerate() {
                    let it = self.check_expr(idx);
                    if let Some((_, expected)) = m.params.get(i) {
                        if !self.is_semantically_assignable(&it, expected)
                            && !matches!(it, Ty::Dyn | Ty::Error)
                        {
                            self.err(
                                format!(
                                    "indexer argument type mismatch: {} vs {}",
                                    it, expected
                                ),
                                span,
                            );
                        }
                    }
                }
                return m.ret;
            }
        }
        let allow_any_key = matches!(
            &ot,
            Ty::Generic { name, .. } if name == "Dictionary"
        ) || matches!(&ot, Ty::Dyn | Ty::Error | Ty::Named(_));
        for idx in indices {
            let it = self.check_expr(idx);
            if allow_any_key {
                continue;
            }
            if matches!(it, Ty::String) {
                continue; // string keys for dict-like
            }
            if !it.is_integral() && !matches!(it, Ty::Dyn | Ty::Error) {
                self.err(
                    format!("index must be integral, found '{}'", it),
                    idx.span(),
                );
            }
        }
        match ot {
            Ty::Array { elem, .. } => *elem,
            Ty::String => Ty::Char,
            Ty::Generic { name, args } if name == "List" => {
                args.into_iter().next().unwrap_or(Ty::Dyn)
            }
            Ty::Generic { name, args } if name == "Dictionary" => {
                args.get(1).cloned().unwrap_or(Ty::Dyn)
            }
            Ty::Dyn | Ty::Error => Ty::Dyn,
            Ty::Named(_) => Ty::Dyn,
            other => {
                self.err(format!("type '{}' is not indexable", other), span);
                Ty::Error
            }
        }
    }

    fn check_new(
        &mut self,
        ty_ref: &TypeRef,
        args: &[Arg],
        init: &[(String, Expr)],
        span: Span,
    ) -> Ty {
        let ty = self.resolve_type_ref(ty_ref);
        if ty_ref.is_array {
            for (_, e) in init {
                let _ = self.check_expr(e);
            }
            for a in args {
                let _ = self.check_expr(&a.value);
            }
            return ty;
        }

        let arg_tys: Vec<Ty> = args.iter().map(|a| self.check_expr(&a.value)).collect();

        let type_name = match &ty {
            Ty::Named(n) => n.clone(),
            Ty::Generic { name, .. } => name.clone(),
            other => {
                if other.is_numeric() || matches!(other, Ty::Bool | Ty::Char | Ty::String) {
                    return ty;
                }
                self.err(format!("cannot construct type '{}'", other), span);
                return Ty::Error;
            }
        };

        let td_info = self.types.get(&type_name).cloned();
        if let Some(td) = td_info {
            if td.kind == TypeDefKind::Interface {
                self.err(
                    format!("cannot construct interface '{}'", type_name),
                    span,
                );
                return Ty::Error;
            }
            if td.is_abstract && td.kind == TypeDefKind::Class {
                self.err(
                    format!("cannot construct abstract class '{}'", type_name),
                    span,
                );
            }
            let ctors = td.constructors;
            if ctors.is_empty() {
                if !arg_tys.is_empty() {
                    self.err(
                        format!(
                            "type '{}' has no constructor taking {} argument(s)",
                            type_name,
                            arg_tys.len()
                        ),
                        span,
                    );
                }
            } else {
                let mut matched = false;
                for ctor in &ctors {
                    if ctor.params.len() == arg_tys.len() {
                        let mut ok = true;
                        for ((_, pt), at) in ctor.params.iter().zip(arg_tys.iter()) {
                            let pt = substitute_for_obj(pt, &ty);
                            if !self.is_semantically_assignable(at, &pt) {
                                ok = false;
                                break;
                            }
                        }
                        if ok {
                            matched = true;
                            break;
                        }
                    }
                }
                if !matched {
                    if let Some(ctor) = ctors.iter().find(|c| c.params.len() == arg_tys.len()) {
                        self.check_args(ctor, &arg_tys, span);
                    } else {
                        self.err(
                            format!(
                                "no matching constructor for '{}' with {} argument(s)",
                                type_name,
                                arg_tys.len()
                            ),
                            span,
                        );
                    }
                }
            }
        } else if !is_primitive_name(&type_name) {
            self.warn(
                format!("constructing unknown type '{}'", type_name),
                span,
            );
        }

        for (field, expr) in init {
            if field.is_empty() {
                let et = self.check_expr(expr);
                if let Some(elem) = self.element_type(&ty) {
                    self.expect_assignable(&et, &elem, expr.span(), "collection initializer");
                }
            } else {
                let ft = self.lookup_field_or_prop(&ty, field, span);
                let et = self.check_expr(expr);
                self.expect_assignable(&et, &ft, expr.span(), "object initializer");
            }
        }

        ty
    }

    fn check_array_lit(&mut self, elems: &[Expr], _span: Span) -> Ty {
        if elems.is_empty() {
            return Ty::Array {
                elem: Box::new(Ty::Dyn),
                dims: 1,
            };
        }
        let mut t = self.check_expr(&elems[0]);
        for e in elems.iter().skip(1) {
            let et = self.check_expr(e);
            t = Ty::unify(&t, &et);
        }
        Ty::Array {
            elem: Box::new(t),
            dims: 1,
        }
    }

    fn check_lambda(&mut self, params: &[Param], body: &FunctionBody, _span: Span) -> Ty {
        self.push_scope();
        let mut pts = Vec::new();
        for p in params {
            let ty = self.resolve_type_ref(&p.ty);
            self.declare_local(&p.name, ty.clone(), false, p.span);
            pts.push(ty);
        }
        let ret = match body {
            FunctionBody::Block(b) => {
                let prev = self.expected_return.replace(Ty::Dyn);
                self.check_block(b);
                self.expected_return = prev;
                Ty::Dyn
            }
            FunctionBody::Expr(e) => self.check_expr(e),
        };
        self.pop_scope();
        Ty::Func {
            params: pts,
            ret: Box::new(ret),
        }
    }

    fn expect_assignable(&mut self, from: &Ty, to: &Ty, span: Span, ctx: &str) {
        if self.is_semantically_assignable(from, to) {
            return;
        }
        if matches!(from, Ty::Error) || matches!(to, Ty::Error) {
            return;
        }
        self.err(
            format!(
                "type mismatch in {}: expected '{}', found '{}'",
                ctx, to, from
            ),
            span,
        );
    }

    fn type_category(&self, ty: &Ty) -> TypeCategory {
        match ty {
            Ty::Named(name) => self
                .types
                .get(name)
                .map(|td| match td.kind {
                    TypeDefKind::Struct => TypeCategory::Value,
                    TypeDefKind::Class | TypeDefKind::Interface => TypeCategory::Reference,
                })
                .unwrap_or_else(|| ty.category()),
            Ty::Generic { name, .. } => self
                .types
                .get(name)
                .map(|td| match td.kind {
                    TypeDefKind::Struct => TypeCategory::Value,
                    TypeDefKind::Class | TypeDefKind::Interface => TypeCategory::Reference,
                })
                .unwrap_or_else(|| ty.category()),
            other => other.category(),
        }
    }

    fn can_accept_null(&self, ty: &Ty) -> bool {
        matches!(
            self.type_category(ty),
            TypeCategory::Reference
                | TypeCategory::Nullable
                | TypeCategory::Pointer
                | TypeCategory::Dynamic
                | TypeCategory::Error
        )
    }

    fn is_semantically_assignable(&self, from: &Ty, to: &Ty) -> bool {
        if matches!(from, Ty::Error) || matches!(to, Ty::Error) {
            return true;
        }
        if matches!(to, Ty::Dyn | Ty::Error) || matches!(from, Ty::Dyn | Ty::Error) {
            return true;
        }
        if matches!(from, Ty::Null) {
            return self.can_accept_null(to);
        }
        if let Ty::Nullable(inner) = to {
            return self.is_semantically_assignable(from, inner) || matches!(from, Ty::Null);
        }
        if matches!(from, Ty::Nullable(_)) && !matches!(to, Ty::Nullable(_) | Ty::Dyn | Ty::Error) {
            return false;
        }
        match (from, to) {
            (
                Ty::Generic { name: na, args: aa },
                Ty::Generic { name: nb, args: ab },
            ) if na == nb && aa.len() == ab.len() => {
                return aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(a, b)| self.is_semantically_assignable(a, b) && self.is_semantically_assignable(b, a));
            }
            _ => {}
        }
        from.is_assignable_to(to) || self.is_subtype(from, to)
    }
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Public entry: typecheck a program and return a report.
pub fn typecheck(program: &Program) -> TypeCheckReport {
    typecheck_with_stdlib(program, true)
}

pub fn typecheck_with_stdlib(program: &Program, stdlib_enabled: bool) -> TypeCheckReport {
    TypeChecker::with_stdlib(stdlib_enabled).check(program)
}

/// Typecheck and return Err on first error.
pub fn typecheck_or_err(program: &Program) -> CompileResult<()> {
    typecheck_or_err_with_stdlib(program, true)
}

pub fn typecheck_or_err_with_stdlib(program: &Program, stdlib_enabled: bool) -> CompileResult<()> {
    typecheck_with_stdlib(program, stdlib_enabled).into_result()
}

fn is_primitive_name(n: &str) -> bool {
    match n {
        "void"
        | "bool"
        | "byte"
        | "sbyte"
        | "short"
        | "ushort"
        | "int"
        | "uint"
        | "long"
        | "ulong"
        | "float"
        | "double"
        | "decimal"
        | "char"
        | "string"
        | "dyn"
        | "var"
        | "ptr"
        | "object" => true,
        _ => false,
    }
}

fn is_builtin_name(n: &str) -> bool {
    builtin_functions().iter().any(|(name, _, _)| *name == n)
}

fn wrap_task_return(declared: Ty) -> Ty {
    match &declared {
        Ty::Generic { name, .. } if name == "Task" => declared,
        Ty::Named(n) if n == "Task" => declared,
        Ty::Void => Ty::Generic {
            name: "Task".into(),
            args: vec![Ty::Void],
        },
        other => Ty::Generic {
            name: "Task".into(),
            args: vec![other.clone()],
        },
    }
}

fn unwrap_task_return(declared: &Ty) -> Ty {
    match declared {
        Ty::Generic { name, args } if name == "Task" => {
            args.first().cloned().unwrap_or(Ty::Void)
        }
        Ty::Named(n) if n == "Task" => Ty::Dyn,
        other => other.clone(),
    }
}

fn ty_from_name(n: &str) -> Ty {
    match n {
        "int" => Ty::Int,
        "float" => Ty::Float,
        "double" => Ty::Double,
        "long" => Ty::Long,
        "bool" => Ty::Bool,
        "string" => Ty::String,
        _ => Ty::Named(n.into()),
    }
}

fn substitute(ty: &Ty, subst: &HashMap<String, Ty>) -> Ty {
    match ty {
        Ty::TypeParam(n) => subst.get(n).cloned().unwrap_or_else(|| ty.clone()),
        Ty::Generic { name, args } => Ty::Generic {
            name: name.clone(),
            args: args.iter().map(|a| substitute(a, subst)).collect(),
        },
        Ty::Array { elem, dims } => Ty::Array {
            elem: Box::new(substitute(elem, subst)),
            dims: *dims,
        },
        Ty::Nullable(inner) => Ty::Nullable(Box::new(substitute(inner, subst))),
        Ty::Ptr(inner) => Ty::Ptr(Box::new(substitute(inner, subst))),
        Ty::Func { params, ret } => Ty::Func {
            params: params.iter().map(|p| substitute(p, subst)).collect(),
            ret: Box::new(substitute(ret, subst)),
        },
        other => other.clone(),
    }
}

fn substitute_for_obj(ty: &Ty, obj: &Ty) -> Ty {
    let mut subst = HashMap::new();
    if let Ty::Generic { args, .. } = obj {
        if args.len() == 1 {
            subst.insert("T".into(), args[0].clone());
        } else if args.len() == 2 {
            subst.insert("TKey".into(), args[0].clone());
            subst.insert("TValue".into(), args[1].clone());
            subst.insert("T".into(), args[0].clone());
            subst.insert("U".into(), args[1].clone());
        }
    }
    substitute(ty, &subst)
}
