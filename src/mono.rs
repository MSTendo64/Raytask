//! Compile-time generics monomorphization.
//!
//! Collects concrete instantiations (`foo<int>`, `new Box<string>()`), clones
//! generic templates with type parameters substituted, and rewrites call/new
//! sites to the mangled specialized names.

use crate::ast::*;
use crate::span::Span;
use std::collections::{HashMap, HashSet, VecDeque};

/// Stdlib / builtin generics stay erased at runtime — do not specialize.
fn is_builtin_generic(name: &str) -> bool {
    matches!(
        name,
        "List"
            | "Dictionary"
            | "Dict"
            | "Set"
            | "Queue"
            | "Stack"
            | "Task"
            | "Result"
            | "Option"
            | "ptr"
            | "Array"
            | "Span"
            | "Func"
            | "Action"
            | "Nullable"
    )
}

pub fn mangle_type(ty: &TypeRef) -> String {
    let mut s = sanitize(&ty.name);
    for a in &ty.args {
        s.push_str("__");
        s.push_str(&mangle_type(a));
    }
    if ty.is_array {
        for _ in 0..ty.array_dims.max(1) {
            s.push_str("__arr");
        }
    }
    if ty.nullable {
        s.push_str("__n");
    }
    s
}

pub fn mangle_instance(base: &str, args: &[TypeRef]) -> String {
    let mut s = sanitize(base);
    for a in args {
        s.push_str("__");
        s.push_str(&mangle_type(a));
    }
    s
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

fn subst_type(ty: &TypeRef, map: &HashMap<String, TypeRef>) -> TypeRef {
    if ty.args.is_empty() {
        if let Some(repl) = map.get(&ty.name) {
            let mut t = repl.clone();
            t.nullable = t.nullable || ty.nullable;
            if ty.is_array {
                t.is_array = true;
                t.array_dims = ty.array_dims.max(t.array_dims.max(1));
            }
            return t;
        }
    }
    TypeRef {
        name: ty.name.clone(),
        args: ty.args.iter().map(|a| subst_type(a, map)).collect(),
        nullable: ty.nullable,
        is_array: ty.is_array,
        array_dims: ty.array_dims,
        span: ty.span,
    }
}

fn build_subst(params: &[String], args: &[TypeRef]) -> HashMap<String, TypeRef> {
    let mut map = HashMap::new();
    for (i, p) in params.iter().enumerate() {
        if let Some(a) = args.get(i) {
            map.insert(p.clone(), a.clone());
        }
    }
    map
}

/// Monomorphize a program. Generic templates without concrete uses are dropped.
pub fn monomorphize(program: Program) -> Program {
    let mut mono = Mono::new();
    mono.run(program)
}

struct Mono {
    fn_templates: HashMap<String, FunctionDecl>,
    class_templates: HashMap<String, ClassDecl>,
    struct_templates: HashMap<String, StructDecl>,
    specialized_fns: HashMap<String, FunctionDecl>,
    specialized_classes: HashMap<String, ClassDecl>,
    specialized_structs: HashMap<String, StructDecl>,
    queue: VecDeque<(Kind, String, Vec<TypeRef>)>,
    done: HashSet<String>,
}

#[derive(Clone, Copy)]
enum Kind {
    Func,
    Class,
    Struct,
}

impl Mono {
    fn new() -> Self {
        Self {
            fn_templates: HashMap::new(),
            class_templates: HashMap::new(),
            struct_templates: HashMap::new(),
            specialized_fns: HashMap::new(),
            specialized_classes: HashMap::new(),
            specialized_structs: HashMap::new(),
            queue: VecDeque::new(),
            done: HashSet::new(),
        }
    }

    fn run(&mut self, program: Program) -> Program {
        let mut kept = Vec::new();
        for item in program.items {
            self.ingest_item(item, &mut kept);
        }

        // Seed queue from non-generic bodies
        for item in &kept {
            self.collect_item(item);
        }

        while let Some((kind, name, args)) = self.queue.pop_front() {
            let mangled = mangle_instance(&name, &args);
            if !self.done.insert(mangled.clone()) {
                continue;
            }
            match kind {
                Kind::Func => self.specialize_fn(&name, &args),
                Kind::Class => self.specialize_class(&name, &args),
                Kind::Struct => self.specialize_struct(&name, &args),
            }
        }

        // Rewrite kept items + append specialized
        let mut out: Vec<Item> = kept
            .into_iter()
            .map(|i| self.rewrite_item(i))
            .collect();
        for f in self.specialized_fns.values() {
            out.push(Item::Function(f.clone()));
        }
        for c in self.specialized_classes.values() {
            out.push(Item::Class(c.clone()));
        }
        for s in self.specialized_structs.values() {
            out.push(Item::Struct(s.clone()));
        }
        // Rewrite specialized bodies too (calls inside them)
        out = out.into_iter().map(|i| self.rewrite_item(i)).collect();

        Program { items: out }
    }

    fn ingest_item(&mut self, item: Item, kept: &mut Vec<Item>) {
        match item {
            Item::Attribute(a, inner) => {
                let mut nested = Vec::new();
                self.ingest_item(*inner, &mut nested);
                for n in nested {
                    kept.push(Item::Attribute(a.clone(), Box::new(n)));
                }
            }
            Item::Namespace(mut ns) => {
                let mut inner_kept = Vec::new();
                for i in std::mem::take(&mut ns.items) {
                    self.ingest_item(i, &mut inner_kept);
                }
                ns.items = inner_kept;
                kept.push(Item::Namespace(ns));
            }
            Item::Function(f) => {
                if !f.type_params.is_empty() && !is_builtin_generic(&f.name) {
                    self.fn_templates.insert(f.name.clone(), f);
                } else {
                    kept.push(Item::Function(f));
                }
            }
            Item::Class(c) => {
                if !c.type_params.is_empty() && !is_builtin_generic(&c.name) {
                    self.class_templates.insert(c.name.clone(), c);
                } else {
                    kept.push(Item::Class(c));
                }
            }
            Item::Struct(s) => {
                if !s.type_params.is_empty() && !is_builtin_generic(&s.name) {
                    self.struct_templates.insert(s.name.clone(), s);
                } else {
                    kept.push(Item::Struct(s));
                }
            }
            other => kept.push(other),
        }
    }

    fn request(&mut self, kind: Kind, name: &str, args: &[TypeRef]) {
        if args.is_empty() || is_builtin_generic(name) {
            return;
        }
        let exists = match kind {
            Kind::Func => self.fn_templates.contains_key(name),
            Kind::Class => self.class_templates.contains_key(name),
            Kind::Struct => self.struct_templates.contains_key(name),
        };
        if !exists {
            return;
        }
        self.queue
            .push_back((kind, name.to_string(), args.to_vec()));
    }

    fn specialize_fn(&mut self, name: &str, args: &[TypeRef]) {
        let Some(tmpl) = self.fn_templates.get(name).cloned() else {
            return;
        };
        let map = build_subst(&tmpl.type_params, args);
        let mangled = mangle_instance(name, args);
        let mut f = tmpl;
        f.name = mangled.clone();
        f.type_params.clear();
        f.constraints.clear();
        f.return_type = subst_type(&f.return_type, &map);
        for p in &mut f.params {
            p.ty = subst_type(&p.ty, &map);
            if let Some(d) = &mut p.default {
                *d = subst_expr(d, &map);
            }
        }
        if let Some(body) = &mut f.body {
            *body = subst_fn_body(body, &map);
        }
        self.collect_fn_body(&f);
        self.specialized_fns.insert(mangled, f);
    }

    fn specialize_class(&mut self, name: &str, args: &[TypeRef]) {
        let Some(tmpl) = self.class_templates.get(name).cloned() else {
            return;
        };
        let map = build_subst(&tmpl.type_params, args);
        let mangled = mangle_instance(name, args);
        let mut c = tmpl;
        c.name = mangled.clone();
        c.type_params.clear();
        c.constraints.clear();
        c.bases = c
            .bases
            .iter()
            .map(|b| subst_type(b, &map))
            .collect();
        c.members = c
            .members
            .iter()
            .map(|m| subst_member(m, &map))
            .collect();
        self.collect_class(&c);
        self.specialized_classes.insert(mangled, c);
    }

    fn specialize_struct(&mut self, name: &str, args: &[TypeRef]) {
        let Some(tmpl) = self.struct_templates.get(name).cloned() else {
            return;
        };
        let map = build_subst(&tmpl.type_params, args);
        let mangled = mangle_instance(name, args);
        let mut s = tmpl;
        s.name = mangled.clone();
        s.type_params.clear();
        s.members = s
            .members
            .iter()
            .map(|m| subst_member(m, &map))
            .collect();
        self.collect_struct(&s);
        self.specialized_structs.insert(mangled, s);
    }

    fn collect_item(&mut self, item: &Item) {
        match item {
            Item::Attribute(_, inner) => self.collect_item(inner),
            Item::Namespace(ns) => {
                for i in &ns.items {
                    self.collect_item(i);
                }
            }
            Item::Function(f) => self.collect_fn_body(f),
            Item::Class(c) => self.collect_class(c),
            Item::Struct(s) => self.collect_struct(s),
            Item::Const(c) => self.collect_expr(&c.value),
            _ => {}
        }
    }

    fn collect_fn_body(&mut self, f: &FunctionDecl) {
        if let Some(body) = &f.body {
            match body {
                FunctionBody::Block(b) => self.collect_block(b),
                FunctionBody::Expr(e) => self.collect_expr(e),
            }
        }
        for p in &f.params {
            if let Some(d) = &p.default {
                self.collect_expr(d);
            }
        }
    }

    fn collect_class(&mut self, c: &ClassDecl) {
        for m in &c.members {
            self.collect_member(m);
        }
    }

    fn collect_struct(&mut self, s: &StructDecl) {
        for m in &s.members {
            self.collect_member(m);
        }
    }

    fn collect_member(&mut self, m: &Member) {
        match m {
            Member::Field(f) => {
                if let Some(i) = &f.init {
                    self.collect_expr(i);
                }
            }
            Member::Method(f) => self.collect_fn_body(f),
            Member::Constructor(ctor) => {
                self.collect_block(&ctor.body);
                for a in &ctor.base_args {
                    self.collect_expr(a);
                }
            }
            Member::Destructor(d) => self.collect_block(&d.body),
            Member::Property(p) => {
                if let Some(g) = &p.getter {
                    self.collect_block(g);
                }
                if let Some(s) = &p.setter {
                    self.collect_block(s);
                }
            }
            Member::Indexer(i) => {
                if let Some(g) = &i.getter {
                    self.collect_block(g);
                }
                if let Some(s) = &i.setter {
                    self.collect_block(s);
                }
            }
            Member::Operator(o) => self.collect_block(&o.body),
        }
    }

    fn collect_block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.collect_stmt(s);
        }
    }

    fn collect_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Expr(e) | Stmt::Return(Some(e), _) | Stmt::Throw(e, _) => self.collect_expr(e),
            Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Decl(d) => {
                if let Some(i) = &d.init {
                    self.collect_expr(i);
                }
            }
            Stmt::Using { decl, body, .. } => {
                if let Some(i) = &decl.init {
                    self.collect_expr(i);
                }
                self.collect_block(body);
            }
            Stmt::Const(c) => self.collect_expr(&c.value),
            Stmt::Block(b) | Stmt::Unsafe(b, _) => self.collect_block(b),
            Stmt::If {
                cond,
                then_block,
                else_branch,
                ..
            } => {
                self.collect_expr(cond);
                self.collect_block(then_block);
                if let Some(e) = else_branch {
                    match e {
                        ElseBranch::Block(b) => self.collect_block(b),
                        ElseBranch::If(s) => self.collect_stmt(s),
                    }
                }
            }
            Stmt::While { cond, body, .. } | Stmt::DoWhile { body, cond, .. } => {
                self.collect_expr(cond);
                self.collect_block(body);
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
                ..
            } => {
                if let Some(i) = init {
                    self.collect_stmt(i);
                }
                if let Some(c) = cond {
                    self.collect_expr(c);
                }
                if let Some(s) = step {
                    self.collect_expr(s);
                }
                self.collect_block(body);
            }
            Stmt::Foreach { iter, body, .. } => {
                self.collect_expr(iter);
                self.collect_block(body);
            }
            Stmt::Switch { expr, cases, .. } => {
                self.collect_expr(expr);
                for case in cases {
                    for pat in &case.patterns {
                        match pat {
                            crate::ast::SwitchPattern::Expr(e) => self.collect_expr(e),
                            crate::ast::SwitchPattern::Range(lo, hi) => {
                                self.collect_expr(lo);
                                self.collect_expr(hi);
                            }
                        }
                    }
                    if let Some(g) = &case.guard { self.collect_expr(g); }
                    for s in &case.body {
                        self.collect_stmt(s);
                    }
                }
            }
            Stmt::Match { expr, arms, .. } => {
                self.collect_expr(expr);
                for arm in arms {
                    self.collect_expr(&arm.body);
                }
            }
            Stmt::Try {
                body,
                catches,
                finally,
                ..
            } => {
                self.collect_block(body);
                for c in catches {
                    self.collect_block(&c.body);
                }
                if let Some(f) = finally {
                    self.collect_block(f);
                }
            }
        }
    }

    fn collect_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Call {
                callee,
                type_args,
                args,
                ..
            } => {
                if let Expr::Ident(name, _) = callee.as_ref() {
                    if !type_args.is_empty() {
                        self.request(Kind::Func, name, type_args);
                    }
                }
                self.collect_expr(callee);
                for a in args {
                    self.collect_expr(&a.value);
                }
            }
            Expr::New { ty, args, init, .. } => {
                if !ty.args.is_empty() {
                    self.request(Kind::Class, &ty.name, &ty.args);
                    self.request(Kind::Struct, &ty.name, &ty.args);
                }
                for a in args {
                    self.collect_expr(&a.value);
                }
                for (_, e) in init {
                    self.collect_expr(e);
                }
            }
            Expr::Binary { left, right, .. }
            | Expr::Assign {
                target: left,
                value: right,
                ..
            } => {
                self.collect_expr(left);
                self.collect_expr(right);
            }
            Expr::Unary { expr, .. }
            | Expr::Await(expr, _)
            | Expr::Deref(expr, _)
            | Expr::AddressOf(expr, _)
            | Expr::Try(expr, _)
            | Expr::Cast { expr, .. }
            | Expr::Is { expr, .. }
            | Expr::As { expr, .. }
            | Expr::Grouped(expr, _) => self.collect_expr(expr),
            Expr::Member { object, .. } | Expr::PtrMember { object, .. } => {
                self.collect_expr(object);
            }
            Expr::Index { object, indices, .. } => {
                self.collect_expr(object);
                for i in indices {
                    self.collect_expr(i);
                }
            }
            Expr::ArrayLit(elems, _) => {
                for e in elems {
                    self.collect_expr(e);
                }
            }
            Expr::Lambda { body, .. } => match body {
                FunctionBody::Block(b) => self.collect_block(b),
                FunctionBody::Expr(e) => self.collect_expr(e),
            },
            Expr::Ternary {
                cond,
                then_expr,
                else_expr,
                ..
            } => {
                self.collect_expr(cond);
                self.collect_expr(then_expr);
                self.collect_expr(else_expr);
            }
            Expr::Interpolated(parts, _) => {
                for p in parts {
                    if let InterpPart::Expr(e) = p {
                        self.collect_expr(e);
                    }
                }
            }
            Expr::TypeOf(ty, _) => {
                if !ty.args.is_empty() {
                    self.request(Kind::Class, &ty.name, &ty.args);
                    self.request(Kind::Struct, &ty.name, &ty.args);
                }
            }
            _ => {}
        }
    }

    fn rewrite_item(&self, item: Item) -> Item {
        match item {
            Item::Attribute(a, inner) => {
                Item::Attribute(a, Box::new(self.rewrite_item(*inner)))
            }
            Item::Namespace(mut ns) => {
                ns.items = ns
                    .items
                    .into_iter()
                    .map(|i| self.rewrite_item(i))
                    .collect();
                Item::Namespace(ns)
            }
            Item::Function(mut f) => {
                if let Some(body) = f.body.take() {
                    f.body = Some(self.rewrite_fn_body(body));
                }
                Item::Function(f)
            }
            Item::Class(mut c) => {
                c.members = c
                    .members
                    .into_iter()
                    .map(|m| self.rewrite_member(m))
                    .collect();
                Item::Class(c)
            }
            Item::Struct(mut s) => {
                s.members = s
                    .members
                    .into_iter()
                    .map(|m| self.rewrite_member(m))
                    .collect();
                Item::Struct(s)
            }
            Item::Const(mut c) => {
                c.value = self.rewrite_expr(c.value);
                Item::Const(c)
            }
            other => other,
        }
    }

    fn rewrite_member(&self, m: Member) -> Member {
        match m {
            Member::Method(mut f) => {
                if let Some(body) = f.body.take() {
                    f.body = Some(self.rewrite_fn_body(body));
                }
                Member::Method(f)
            }
            Member::Constructor(mut ctor) => {
                ctor.body = self.rewrite_block(ctor.body);
                ctor.base_args = ctor
                    .base_args
                    .into_iter()
                    .map(|e| self.rewrite_expr(e))
                    .collect();
                Member::Constructor(ctor)
            }
            Member::Destructor(mut d) => {
                d.body = self.rewrite_block(d.body);
                Member::Destructor(d)
            }
            Member::Property(mut p) => {
                if let Some(g) = p.getter.take() {
                    p.getter = Some(self.rewrite_block(g));
                }
                if let Some(s) = p.setter.take() {
                    p.setter = Some(self.rewrite_block(s));
                }
                Member::Property(p)
            }
            Member::Indexer(mut i) => {
                if let Some(g) = i.getter.take() {
                    i.getter = Some(self.rewrite_block(g));
                }
                if let Some(s) = i.setter.take() {
                    i.setter = Some(self.rewrite_block(s));
                }
                Member::Indexer(i)
            }
            Member::Operator(mut o) => {
                o.body = self.rewrite_block(o.body);
                Member::Operator(o)
            }
            Member::Field(mut f) => {
                if let Some(i) = f.init.take() {
                    f.init = Some(self.rewrite_expr(i));
                }
                Member::Field(f)
            }
        }
    }

    fn rewrite_fn_body(&self, body: FunctionBody) -> FunctionBody {
        match body {
            FunctionBody::Block(b) => FunctionBody::Block(self.rewrite_block(b)),
            FunctionBody::Expr(e) => FunctionBody::Expr(Box::new(self.rewrite_expr(*e))),
        }
    }

    fn rewrite_block(&self, mut b: Block) -> Block {
        b.stmts = b
            .stmts
            .into_iter()
            .map(|s| self.rewrite_stmt(s))
            .collect();
        b
    }

    fn rewrite_stmt(&self, stmt: Stmt) -> Stmt {
        match stmt {
            Stmt::Expr(e) => Stmt::Expr(self.rewrite_expr(e)),
            Stmt::Return(Some(e), s) => Stmt::Return(Some(self.rewrite_expr(e)), s),
            Stmt::Decl(mut d) => {
                if let Some(i) = d.init.take() {
                    d.init = Some(self.rewrite_expr(i));
                }
                Stmt::Decl(d)
            }
            Stmt::Const(mut c) => {
                c.value = self.rewrite_expr(c.value);
                Stmt::Const(c)
            }
            Stmt::Block(b) => Stmt::Block(self.rewrite_block(b)),
            Stmt::If {
                cond,
                then_block,
                else_branch,
                span,
            } => Stmt::If {
                cond: self.rewrite_expr(cond),
                then_block: self.rewrite_block(then_block),
                else_branch: else_branch.map(|e| match e {
                    ElseBranch::Block(b) => ElseBranch::Block(self.rewrite_block(b)),
                    ElseBranch::If(s) => ElseBranch::If(Box::new(self.rewrite_stmt(*s))),
                }),
                span,
            },
            Stmt::While { cond, body, span } => Stmt::While {
                cond: self.rewrite_expr(cond),
                body: self.rewrite_block(body),
                span,
            },
            Stmt::DoWhile { body, cond, span } => Stmt::DoWhile {
                body: self.rewrite_block(body),
                cond: self.rewrite_expr(cond),
                span,
            },
            Stmt::For {
                init,
                cond,
                step,
                body,
                span,
            } => Stmt::For {
                init: init.map(|i| Box::new(self.rewrite_stmt(*i))),
                cond: cond.map(|c| self.rewrite_expr(c)),
                step: step.map(|s| self.rewrite_expr(s)),
                body: self.rewrite_block(body),
                span,
            },
            Stmt::Foreach {
                var_name,
                index_name,
                iter,
                body,
                span,
            } => Stmt::Foreach {
                var_name,
                index_name,
                iter: self.rewrite_expr(iter),
                body: self.rewrite_block(body),
                span,
            },
            Stmt::Throw(e, s) => Stmt::Throw(self.rewrite_expr(e), s),
            Stmt::Try {
                body,
                catches,
                finally,
                span,
            } => Stmt::Try {
                body: self.rewrite_block(body),
                catches: catches
                    .into_iter()
                    .map(|mut c| {
                        c.body = self.rewrite_block(c.body);
                        c
                    })
                    .collect(),
                finally: finally.map(|f| self.rewrite_block(f)),
                span,
            },
            Stmt::Using { mut decl, body, span } => {
                if let Some(i) = decl.init.take() {
                    decl.init = Some(self.rewrite_expr(i));
                }
                Stmt::Using {
                    decl,
                    body: self.rewrite_block(body),
                    span,
                }
            }
            Stmt::Unsafe(b, s) => Stmt::Unsafe(self.rewrite_block(b), s),
            Stmt::Switch { expr, cases, span } => Stmt::Switch {
                expr: self.rewrite_expr(expr),
                cases: cases
                    .into_iter()
                    .map(|c| SwitchCase {
                        patterns: c.patterns.into_iter().map(|p| match p {
                            crate::ast::SwitchPattern::Expr(e) => crate::ast::SwitchPattern::Expr(self.rewrite_expr(e)),
                            crate::ast::SwitchPattern::Range(lo, hi) => crate::ast::SwitchPattern::Range(self.rewrite_expr(lo), self.rewrite_expr(hi)),
                        }).collect(),
                        pattern_bind: c.pattern_bind,
                        guard: c.guard.map(|g| self.rewrite_expr(g)),
                        body: c
                            .body
                            .into_iter()
                            .map(|s| self.rewrite_stmt(s))
                            .collect(),
                    })
                    .collect(),
                span,
            },
            Stmt::Match { expr, arms, span } => Stmt::Match {
                expr: self.rewrite_expr(expr),
                arms: arms
                    .into_iter()
                    .map(|a| MatchArm {
                        pattern: a.pattern,
                        bind: a.bind,
                        body: self.rewrite_expr(a.body),
                    })
                    .collect(),
                span,
            },
            other => other,
        }
    }

    fn rewrite_expr(&self, expr: Expr) -> Expr {
        match expr {
            Expr::Call {
                callee,
                type_args,
                args,
                span,
            } => {
                let args: Vec<_> = args
                    .into_iter()
                    .map(|mut a| {
                        a.value = self.rewrite_expr(a.value);
                        a
                    })
                    .collect();
                if let Expr::Ident(name, ispan) = callee.as_ref() {
                    if !type_args.is_empty()
                        && (self.fn_templates.contains_key(name)
                            || self.specialized_fns.contains_key(&mangle_instance(name, &type_args)))
                    {
                        let mangled = mangle_instance(name, &type_args);
                        return Expr::Call {
                            callee: Box::new(Expr::Ident(mangled, *ispan)),
                            type_args: vec![],
                            args,
                            span,
                        };
                    }
                }
                Expr::Call {
                    callee: Box::new(self.rewrite_expr(*callee)),
                    type_args: vec![],
                    args,
                    span,
                }
            }
            Expr::New {
                mut ty,
                args,
                init,
                span,
            } => {
                if !ty.args.is_empty()
                    && (self.class_templates.contains_key(&ty.name)
                        || self.struct_templates.contains_key(&ty.name)
                        || self
                            .specialized_classes
                            .contains_key(&mangle_instance(&ty.name, &ty.args))
                        || self
                            .specialized_structs
                            .contains_key(&mangle_instance(&ty.name, &ty.args)))
                {
                    let mangled = mangle_instance(&ty.name, &ty.args);
                    ty.name = mangled;
                    ty.args.clear();
                }
                Expr::New {
                    ty,
                    args: args
                        .into_iter()
                        .map(|mut a| {
                            a.value = self.rewrite_expr(a.value);
                            a
                        })
                        .collect(),
                    init: init
                        .into_iter()
                        .map(|(n, e)| (n, self.rewrite_expr(e)))
                        .collect(),
                    span,
                }
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => Expr::Binary {
                left: Box::new(self.rewrite_expr(*left)),
                op,
                right: Box::new(self.rewrite_expr(*right)),
                span,
            },
            Expr::Assign {
                target,
                op,
                value,
                span,
            } => Expr::Assign {
                target: Box::new(self.rewrite_expr(*target)),
                op,
                value: Box::new(self.rewrite_expr(*value)),
                span,
            },
            Expr::Unary { op, expr, span } => Expr::Unary {
                op,
                expr: Box::new(self.rewrite_expr(*expr)),
                span,
            },
            Expr::Member {
                object,
                field,
                null_safe,
                span,
            } => Expr::Member {
                object: Box::new(self.rewrite_expr(*object)),
                field,
                null_safe,
                span,
            },
            Expr::Index {
                object,
                indices,
                span,
            } => Expr::Index {
                object: Box::new(self.rewrite_expr(*object)),
                indices: indices
                    .into_iter()
                    .map(|i| self.rewrite_expr(i))
                    .collect(),
                span,
            },
            Expr::ArrayLit(elems, span) => Expr::ArrayLit(
                elems.into_iter().map(|e| self.rewrite_expr(e)).collect(),
                span,
            ),
            Expr::Lambda { params, body, span } => Expr::Lambda {
                params,
                body: self.rewrite_fn_body(body),
                span,
            },
            Expr::Ternary {
                cond,
                then_expr,
                else_expr,
                span,
            } => Expr::Ternary {
                cond: Box::new(self.rewrite_expr(*cond)),
                then_expr: Box::new(self.rewrite_expr(*then_expr)),
                else_expr: Box::new(self.rewrite_expr(*else_expr)),
                span,
            },
            Expr::Cast { ty, expr, span } => Expr::Cast {
                ty,
                expr: Box::new(self.rewrite_expr(*expr)),
                span,
            },
            Expr::Is { expr, ty, span } => Expr::Is {
                expr: Box::new(self.rewrite_expr(*expr)),
                ty,
                span,
            },
            Expr::As { expr, ty, span } => Expr::As {
                expr: Box::new(self.rewrite_expr(*expr)),
                ty,
                span,
            },
            Expr::Await(e, s) => Expr::Await(Box::new(self.rewrite_expr(*e)), s),
            Expr::Deref(e, s) => Expr::Deref(Box::new(self.rewrite_expr(*e)), s),
            Expr::AddressOf(e, s) => Expr::AddressOf(Box::new(self.rewrite_expr(*e)), s),
            Expr::Try(e, s) => Expr::Try(Box::new(self.rewrite_expr(*e)), s),
            Expr::Grouped(e, s) => Expr::Grouped(Box::new(self.rewrite_expr(*e)), s),
            Expr::PtrMember {
                object,
                field,
                span,
            } => Expr::PtrMember {
                object: Box::new(self.rewrite_expr(*object)),
                field,
                span,
            },
            Expr::Interpolated(parts, span) => Expr::Interpolated(
                parts
                    .into_iter()
                    .map(|p| match p {
                        InterpPart::Literal(s) => InterpPart::Literal(s),
                        InterpPart::Expr(e) => InterpPart::Expr(self.rewrite_expr(e)),
                    })
                    .collect(),
                span,
            ),
            other => other,
        }
    }
}

fn subst_fn_body(body: &FunctionBody, map: &HashMap<String, TypeRef>) -> FunctionBody {
    match body {
        FunctionBody::Block(b) => FunctionBody::Block(subst_block(b, map)),
        FunctionBody::Expr(e) => FunctionBody::Expr(Box::new(subst_expr(e, map))),
    }
}

fn subst_member(m: &Member, map: &HashMap<String, TypeRef>) -> Member {
    match m {
        Member::Field(f) => Member::Field(FieldDecl {
            access: f.access.clone(),
            is_const: f.is_const,
            ty: f.ty.as_ref().map(|t| subst_type(t, map)),
            name: f.name.clone(),
            init: f.init.as_ref().map(|e| subst_expr(e, map)),
            span: f.span,
        }),
        Member::Method(f) => {
            let mut f = f.clone();
            f.return_type = subst_type(&f.return_type, map);
            for p in &mut f.params {
                p.ty = subst_type(&p.ty, map);
                if let Some(d) = &mut p.default {
                    *d = subst_expr(d, map);
                }
            }
            if let Some(body) = &mut f.body {
                *body = subst_fn_body(body, map);
            }
            Member::Method(f)
        }
        Member::Constructor(ctor) => Member::Constructor(ConstructorDecl {
            params: ctor
                .params
                .iter()
                .map(|p| {
                    let mut p = p.clone();
                    p.ty = subst_type(&p.ty, map);
                    p
                })
                .collect(),
            base_args: ctor
                .base_args
                .iter()
                .map(|e| subst_expr(e, map))
                .collect(),
            body: subst_block(&ctor.body, map),
            span: ctor.span,
        }),
        Member::Destructor(d) => Member::Destructor(DestructorDecl {
            body: subst_block(&d.body, map),
            span: d.span,
        }),
        Member::Property(p) => Member::Property(PropertyDecl {
            access: p.access.clone(),
            name: p.name.clone(),
            ty: subst_type(&p.ty, map),
            getter: p.getter.as_ref().map(|g| subst_block(g, map)),
            setter: p.setter.as_ref().map(|s| subst_block(s, map)),
            auto: p.auto,
            span: p.span,
        }),
        Member::Indexer(i) => Member::Indexer(IndexerDecl {
            ty: subst_type(&i.ty, map),
            params: i
                .params
                .iter()
                .map(|p| {
                    let mut p = p.clone();
                    p.ty = subst_type(&p.ty, map);
                    p
                })
                .collect(),
            getter: i.getter.as_ref().map(|g| subst_block(g, map)),
            setter: i.setter.as_ref().map(|s| subst_block(s, map)),
            span: i.span,
        }),
        Member::Operator(o) => Member::Operator(OperatorDecl {
            op: o.op.clone(),
            params: o
                .params
                .iter()
                .map(|p| {
                    let mut p = p.clone();
                    p.ty = subst_type(&p.ty, map);
                    p
                })
                .collect(),
            return_type: subst_type(&o.return_type, map),
            body: subst_block(&o.body, map),
            span: o.span,
        }),
    }
}

fn subst_block(b: &Block, map: &HashMap<String, TypeRef>) -> Block {
    Block {
        stmts: b.stmts.iter().map(|s| subst_stmt(s, map)).collect(),
        span: b.span,
    }
}

fn subst_stmt(stmt: &Stmt, map: &HashMap<String, TypeRef>) -> Stmt {
    match stmt {
        Stmt::Expr(e) => Stmt::Expr(subst_expr(e, map)),
        Stmt::Return(Some(e), s) => Stmt::Return(Some(subst_expr(e, map)), *s),
        Stmt::Return(None, s) => Stmt::Return(None, *s),
        Stmt::Decl(d) => Stmt::Decl(VarDecl {
            kind: d.kind,
            ty: d.ty.as_ref().map(|t| subst_type(t, map)),
            name: d.name.clone(),
            init: d.init.as_ref().map(|e| subst_expr(e, map)),
            span: d.span,
        }),
        Stmt::Const(c) => Stmt::Const(ConstDecl {
            ty: subst_type(&c.ty, map),
            name: c.name.clone(),
            value: subst_expr(&c.value, map),
            span: c.span,
        }),
        Stmt::Block(b) => Stmt::Block(subst_block(b, map)),
        Stmt::If {
            cond,
            then_block,
            else_branch,
            span,
        } => Stmt::If {
            cond: subst_expr(cond, map),
            then_block: subst_block(then_block, map),
            else_branch: else_branch.as_ref().map(|e| match e {
                ElseBranch::Block(b) => ElseBranch::Block(subst_block(b, map)),
                ElseBranch::If(s) => ElseBranch::If(Box::new(subst_stmt(s, map))),
            }),
            span: *span,
        },
        Stmt::While { cond, body, span } => Stmt::While {
            cond: subst_expr(cond, map),
            body: subst_block(body, map),
            span: *span,
        },
        Stmt::DoWhile { body, cond, span } => Stmt::DoWhile {
            body: subst_block(body, map),
            cond: subst_expr(cond, map),
            span: *span,
        },
        Stmt::For {
            init,
            cond,
            step,
            body,
            span,
        } => Stmt::For {
            init: init.as_ref().map(|i| Box::new(subst_stmt(i, map))),
            cond: cond.as_ref().map(|c| subst_expr(c, map)),
            step: step.as_ref().map(|s| subst_expr(s, map)),
            body: subst_block(body, map),
            span: *span,
        },
        Stmt::Foreach {
            var_name,
            index_name,
            iter,
            body,
            span,
        } => Stmt::Foreach {
            var_name: var_name.clone(),
            index_name: index_name.clone(),
            iter: subst_expr(iter, map),
            body: subst_block(body, map),
            span: *span,
        },
        Stmt::Throw(e, s) => Stmt::Throw(subst_expr(e, map), *s),
        Stmt::Try {
            body,
            catches,
            finally,
            span,
        } => Stmt::Try {
            body: subst_block(body, map),
            catches: catches
                .iter()
                .map(|c| CatchClause {
                    exception_type: c.exception_type.as_ref().map(|t| subst_type(t, map)),
                    name: c.name.clone(),
                    body: subst_block(&c.body, map),
                })
                .collect(),
            finally: finally.as_ref().map(|f| subst_block(f, map)),
            span: *span,
        },
        Stmt::Using { decl, body, span } => Stmt::Using {
            decl: VarDecl {
                kind: decl.kind,
                ty: decl.ty.as_ref().map(|t| subst_type(t, map)),
                name: decl.name.clone(),
                init: decl.init.as_ref().map(|e| subst_expr(e, map)),
                span: decl.span,
            },
            body: subst_block(body, map),
            span: *span,
        },
        Stmt::Unsafe(b, s) => Stmt::Unsafe(subst_block(b, map), *s),
        Stmt::Break(s) => Stmt::Break(*s),
        Stmt::Continue(s) => Stmt::Continue(*s),
        Stmt::Switch { expr, cases, span } => Stmt::Switch {
            expr: subst_expr(expr, map),
            cases: cases
                .iter()
                .map(|c| SwitchCase {
                    patterns: c.patterns.iter().map(|p| match p {
                        crate::ast::SwitchPattern::Expr(e) => crate::ast::SwitchPattern::Expr(subst_expr(e, map)),
                        crate::ast::SwitchPattern::Range(lo, hi) => crate::ast::SwitchPattern::Range(subst_expr(lo, map), subst_expr(hi, map)),
                    }).collect(),
                    pattern_bind: c.pattern_bind.clone(),
                    guard: c.guard.as_ref().map(|g| subst_expr(g, map)),
                    body: c.body.iter().map(|s| subst_stmt(s, map)).collect(),
                })
                .collect(),
            span: *span,
        },
        Stmt::Match { expr, arms, span } => Stmt::Match {
            expr: subst_expr(expr, map),
            arms: arms
                .iter()
                .map(|a| MatchArm {
                    pattern: a.pattern.clone(),
                    bind: a.bind.clone(),
                    body: subst_expr(&a.body, map),
                })
                .collect(),
            span: *span,
        },
    }
}

fn subst_expr(expr: &Expr, map: &HashMap<String, TypeRef>) -> Expr {
    match expr {
        Expr::Call {
            callee,
            type_args,
            args,
            span,
        } => Expr::Call {
            callee: Box::new(subst_expr(callee, map)),
            type_args: type_args.iter().map(|t| subst_type(t, map)).collect(),
            args: args
                .iter()
                .map(|a| Arg {
                    name: a.name.clone(),
                    value: subst_expr(&a.value, map),
                })
                .collect(),
            span: *span,
        },
        Expr::New { ty, args, init, span } => Expr::New {
            ty: subst_type(ty, map),
            args: args
                .iter()
                .map(|a| Arg {
                    name: a.name.clone(),
                    value: subst_expr(&a.value, map),
                })
                .collect(),
            init: init
                .iter()
                .map(|(n, e)| (n.clone(), subst_expr(e, map)))
                .collect(),
            span: *span,
        },
        Expr::Binary {
            left,
            op,
            right,
            span,
        } => Expr::Binary {
            left: Box::new(subst_expr(left, map)),
            op: *op,
            right: Box::new(subst_expr(right, map)),
            span: *span,
        },
        Expr::Assign {
            target,
            op,
            value,
            span,
        } => Expr::Assign {
            target: Box::new(subst_expr(target, map)),
            op: *op,
            value: Box::new(subst_expr(value, map)),
            span: *span,
        },
        Expr::Unary { op, expr, span } => Expr::Unary {
            op: *op,
            expr: Box::new(subst_expr(expr, map)),
            span: *span,
        },
        Expr::Member {
            object,
            field,
            null_safe,
            span,
        } => Expr::Member {
            object: Box::new(subst_expr(object, map)),
            field: field.clone(),
            null_safe: *null_safe,
            span: *span,
        },
        Expr::Index {
            object,
            indices,
            span,
        } => Expr::Index {
            object: Box::new(subst_expr(object, map)),
            indices: indices.iter().map(|i| subst_expr(i, map)).collect(),
            span: *span,
        },
        Expr::ArrayLit(elems, span) => {
            Expr::ArrayLit(elems.iter().map(|e| subst_expr(e, map)).collect(), *span)
        }
        Expr::Lambda { params, body, span } => Expr::Lambda {
            params: params
                .iter()
                .map(|p| {
                    let mut p = p.clone();
                    p.ty = subst_type(&p.ty, map);
                    p
                })
                .collect(),
            body: subst_fn_body(body, map),
            span: *span,
        },
        Expr::Ternary {
            cond,
            then_expr,
            else_expr,
            span,
        } => Expr::Ternary {
            cond: Box::new(subst_expr(cond, map)),
            then_expr: Box::new(subst_expr(then_expr, map)),
            else_expr: Box::new(subst_expr(else_expr, map)),
            span: *span,
        },
        Expr::Cast { ty, expr, span } => Expr::Cast {
            ty: subst_type(ty, map),
            expr: Box::new(subst_expr(expr, map)),
            span: *span,
        },
        Expr::TypeOf(ty, span) => Expr::TypeOf(subst_type(ty, map), *span),
        Expr::Is { expr, ty, span } => Expr::Is {
            expr: Box::new(subst_expr(expr, map)),
            ty: subst_type(ty, map),
            span: *span,
        },
        Expr::As { expr, ty, span } => Expr::As {
            expr: Box::new(subst_expr(expr, map)),
            ty: subst_type(ty, map),
            span: *span,
        },
        Expr::Await(e, s) => Expr::Await(Box::new(subst_expr(e, map)), *s),
        Expr::Deref(e, s) => Expr::Deref(Box::new(subst_expr(e, map)), *s),
        Expr::AddressOf(e, s) => Expr::AddressOf(Box::new(subst_expr(e, map)), *s),
        Expr::Try(e, s) => Expr::Try(Box::new(subst_expr(e, map)), *s),
        Expr::Grouped(e, s) => Expr::Grouped(Box::new(subst_expr(e, map)), *s),
        Expr::PtrMember {
            object,
            field,
            span,
        } => Expr::PtrMember {
            object: Box::new(subst_expr(object, map)),
            field: field.clone(),
            span: *span,
        },
        Expr::Interpolated(parts, span) => Expr::Interpolated(
            parts
                .iter()
                .map(|p| match p {
                    InterpPart::Literal(s) => InterpPart::Literal(s.clone()),
                    InterpPart::Expr(e) => InterpPart::Expr(subst_expr(e, map)),
                })
                .collect(),
            *span,
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mangle_nested() {
        let t = TypeRef {
            name: "List".into(),
            args: vec![TypeRef::named("int", Span::default())],
            nullable: false,
            is_array: false,
            array_dims: 0,
            span: Span::default(),
        };
        assert_eq!(mangle_type(&t), "List__int");
        assert_eq!(mangle_instance("id", &[t]), "id__List__int");
    }
}
