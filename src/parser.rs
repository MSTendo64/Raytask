//! Recursive-descent parser for RayTask.

use crate::ast::*;
use crate::error::{CompileError, CompileResult};
use crate::span::Span;
use crate::token::{Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    pub fn parse(mut self) -> CompileResult<Program> {
        let mut items = Vec::new();
        while !self.is_at_end() {
            items.push(self.parse_item()?);
        }
        Ok(Program { items })
    }

    fn current(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .unwrap_or_else(|| self.tokens.last().expect("empty token stream"))
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.pos - 1]
    }

    fn is_at_end(&self) -> bool {
        matches!(self.current().kind, TokenKind::Eof)
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.pos += 1;
        }
        self.previous()
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.current().kind) == std::mem::discriminant(kind)
    }

    fn check_ident(&self) -> bool {
        matches!(self.current().kind, TokenKind::Ident(_))
    }

    fn match_kind(&mut self, kinds: &[TokenKind]) -> bool {
        for k in kinds {
            if self.check(k) {
                self.advance();
                return true;
            }
        }
        false
    }

    fn expect(&mut self, kind: TokenKind, msg: &str) -> CompileResult<&Token> {
        if self.check(&kind) {
            Ok(self.advance())
        } else {
            Err(CompileError::syntax(
                format!("{}, found {}", msg, self.current().lexeme),
                self.current().span,
            ))
        }
    }

    fn expect_ident(&mut self) -> CompileResult<(String, Span)> {
        match &self.current().kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                let span = self.current().span;
                self.advance();
                Ok((name, span))
            }
            // Allow type keywords as identifiers in some contexts? Prefer strict.
            _ => Err(CompileError::syntax(
                format!("expected identifier, found {}", self.current().lexeme),
                self.current().span,
            )),
        }
    }

    /// Path segment for imports/namespaces — allows keywords like `string`, `async`.
    fn expect_path_segment(&mut self) -> CompileResult<(String, Span)> {
        let span = self.current().span;
        let name = match &self.current().kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                return Ok((name, span));
            }
            TokenKind::String => "string",
            TokenKind::Async => "async",
            TokenKind::Unsafe => "unsafe",
            TokenKind::Int => "int",
            TokenKind::Float => "float",
            TokenKind::Double => "double",
            TokenKind::Bool => "bool",
            TokenKind::Var => "var",
            TokenKind::Dyn => "dyn",
            TokenKind::Void => "void",
            _ => {
                return Err(CompileError::syntax(
                    format!("expected identifier, found {}", self.current().lexeme),
                    span,
                ));
            }
        };
        self.advance();
        Ok((name.into(), span))
    }

    fn peek_kind(&self, offset: usize) -> Option<&TokenKind> {
        self.tokens.get(self.pos + offset).map(|t| &t.kind)
    }

    // ─── Top-level ───────────────────────────────────────────

    fn parse_item(&mut self) -> CompileResult<Item> {
        let attrs = self.parse_attributes()?;

        let item = if self.check(&TokenKind::Import) {
            Item::Import(self.parse_import()?)
        } else if self.check(&TokenKind::Namespace) {
            Item::Namespace(self.parse_namespace()?)
        } else if self.check(&TokenKind::Module) {
            Item::Module(self.parse_module()?)
        } else if self.check(&TokenKind::Const) {
            Item::Const(self.parse_const_decl()?)
        } else {
            let access = self.parse_access();
            let is_abstract = self.match_kind(&[TokenKind::Abstract]);

            if self.check(&TokenKind::Class) {
                Item::Class(self.parse_class(access, is_abstract)?)
            } else if self.check(&TokenKind::Struct) {
                Item::Struct(self.parse_struct(access)?)
            } else if self.check(&TokenKind::Union) {
                Item::Union(self.parse_union(access)?)
            } else if self.check(&TokenKind::Interface) {
                Item::Interface(self.parse_interface(access)?)
            } else {
                // Function or skip abstract if misplaced
                Item::Function(self.parse_function(access, is_abstract, attrs.clone())?)
            }
        };

        if attrs.is_empty() {
            Ok(item)
        } else {
            let item = apply_layout_attributes(item, &attrs);
            let mut wrapped = item;
            for attr in attrs.into_iter().rev() {
                wrapped = Item::Attribute(attr, Box::new(wrapped));
            }
            Ok(wrapped)
        }
    }

    fn parse_attributes(&mut self) -> CompileResult<Vec<Attribute>> {
        let mut attrs = Vec::new();
        while self.check(&TokenKind::LBracket) {
            // Could be attribute or array type — attributes are [name] or [name: value]
            // Look ahead: [ Ident|keyword ( : ...)? ]
            let name_ok = match self.peek_kind(1) {
                Some(TokenKind::Ident(_))
                | Some(TokenKind::Export)
                | Some(TokenKind::Private)
                | Some(TokenKind::Protected)
                | Some(TokenKind::Async)
                | Some(TokenKind::Unsafe) => true,
                _ => false,
            };
            if !name_ok {
                break;
            }
            let start = self.current().span;
            self.advance(); // [
            let name = match &self.current().kind {
                TokenKind::Ident(s) => {
                    let n = s.clone();
                    self.advance();
                    n
                }
                TokenKind::Export => {
                    self.advance();
                    "export".into()
                }
                TokenKind::Private => {
                    self.advance();
                    "private".into()
                }
                TokenKind::Protected => {
                    self.advance();
                    "protected".into()
                }
                TokenKind::Async => {
                    self.advance();
                    "async".into()
                }
                TokenKind::Unsafe => {
                    self.advance();
                    "unsafe".into()
                }
                _ => {
                    return Err(CompileError::syntax(
                        "expected attribute name",
                        self.current().span,
                    ));
                }
            };
            let value = if self.match_kind(&[TokenKind::Colon]) {
                Some(self.parse_expression()?)
            } else {
                None
            };
            self.expect(TokenKind::RBracket, "expected ']' after attribute")?;
            attrs.push(Attribute {
                name,
                value,
                span: start.merge(self.previous().span),
            });
        }
        Ok(attrs)
    }

    fn parse_access(&mut self) -> Access {
        if self.match_kind(&[TokenKind::Export]) {
            Access::Export
        } else if self.match_kind(&[TokenKind::Protected]) {
            Access::Protected
        } else if self.match_kind(&[TokenKind::Private]) {
            Access::Private
        } else {
            Access::Default
        }
    }

    fn parse_import(&mut self) -> CompileResult<ImportDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Import, "expected 'import'")?;
        let path = match &self.current().kind {
            TokenKind::Ident(_) => {
                let mut parts = Vec::new();
                let (p, _) = self.expect_ident()?;
                parts.push(p);
                while self.match_kind(&[TokenKind::Dot]) {
                    let (p, _) = self.expect_path_segment()?;
                    parts.push(p);
                }
                parts.join(".")
            }
            TokenKind::StringLit(s) => {
                let s = s.clone();
                self.advance();
                s
            }
            _ => {
                return Err(CompileError::syntax(
                    "expected import path",
                    self.current().span,
                ));
            }
        };
        // Handle `import path as alias;` or `import alias = path;`
        let mut alias = None;
        if self.check(&TokenKind::As) {
            self.advance();
            let (a, _) = self.expect_ident()?;
            alias = Some(a);
        } else if !path.contains('.') && self.check(&TokenKind::Eq) {
            self.advance();
            let mut parts = Vec::new();
            let (p, _) = self.expect_ident()?;
            parts.push(p);
            while self.match_kind(&[TokenKind::Dot]) {
                let (p, _) = self.expect_ident()?;
                parts.push(p);
            }
            alias = Some(path);
            let real_path = parts.join(".");
            self.expect(TokenKind::Semicolon, "expected ';'")?;
            return Ok(ImportDecl {
                path: real_path,
                alias,
                span: start.merge(self.previous().span),
            });
        }
        self.expect(TokenKind::Semicolon, "expected ';' after import")?;
        Ok(ImportDecl {
            path,
            alias,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_namespace(&mut self) -> CompileResult<NamespaceDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Namespace, "expected 'namespace'")?;
        let mut parts = Vec::new();
        let (p, _) = self.expect_ident()?;
        parts.push(p);
        while self.match_kind(&[TokenKind::Dot]) {
            let (p, _) = self.expect_ident()?;
            parts.push(p);
        }
        let name = parts.join(".");
        let items = if self.match_kind(&[TokenKind::LBrace]) {
            let mut items = Vec::new();
            while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
                items.push(self.parse_item()?);
            }
            self.expect(TokenKind::RBrace, "expected '}'")?;
            items
        } else {
            self.expect(TokenKind::Semicolon, "expected ';' or '{'")?;
            vec![]
        };
        Ok(NamespaceDecl {
            name,
            items,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_module(&mut self) -> CompileResult<ModuleDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Module, "expected 'module'")?;
        let name = match &self.current().kind {
            TokenKind::StringLit(s) => {
                let s = s.clone();
                self.advance();
                s
            }
            _ => self.expect_ident()?.0,
        };
        self.expect(TokenKind::LBrace, "expected '{'")?;
        let mut fields = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let (key, _) = self.expect_ident()?;
            self.expect(TokenKind::Eq, "expected '='")?;
            let value = self.parse_expression()?;
            self.expect(TokenKind::Semicolon, "expected ';'")?;
            fields.push((key, value));
        }
        self.expect(TokenKind::RBrace, "expected '}'")?;
        Ok(ModuleDecl {
            name,
            fields,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_class(&mut self, access: Access, is_abstract: bool) -> CompileResult<ClassDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Class, "expected 'class'")?;
        let (name, _) = self.expect_ident()?;
        let type_params = self.parse_type_params()?;
        let mut bases = Vec::new();
        if self.match_kind(&[TokenKind::Colon]) {
            bases.push(self.parse_type_ref()?);
            while self.match_kind(&[TokenKind::Comma]) {
                bases.push(self.parse_type_ref()?);
            }
        }
        let constraints = self.parse_where_clause()?;
        self.expect(TokenKind::LBrace, "expected '{'")?;
        let mut members = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            members.push(self.parse_member()?);
        }
        self.expect(TokenKind::RBrace, "expected '}'")?;
        Ok(ClassDecl {
            access,
            is_abstract,
            name,
            type_params,
            bases,
            constraints,
            members,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_struct(&mut self, access: Access) -> CompileResult<StructDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Struct, "expected 'struct'")?;
        let (name, _) = self.expect_ident()?;
        let type_params = self.parse_type_params()?;
        self.expect(TokenKind::LBrace, "expected '{'")?;
        let mut members = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            members.push(self.parse_member()?);
        }
        self.expect(TokenKind::RBrace, "expected '}'")?;
        Ok(StructDecl {
            access,
            name,
            type_params,
            members,
            attributes: vec![],
            packed: false,
            align: None,
            repr_c: false,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_union(&mut self, access: Access) -> CompileResult<UnionDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Union, "expected 'union'")?;
        let (name, _) = self.expect_ident()?;
        self.expect(TokenKind::LBrace, "expected '{'")?;
        let mut members = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            members.push(self.parse_member()?);
        }
        self.expect(TokenKind::RBrace, "expected '}'")?;
        Ok(UnionDecl {
            access,
            name,
            members,
            attributes: vec![],
            packed: false,
            align: None,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_interface(&mut self, access: Access) -> CompileResult<InterfaceDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Interface, "expected 'interface'")?;
        let (name, _) = self.expect_ident()?;
        let type_params = self.parse_type_params()?;
        self.expect(TokenKind::LBrace, "expected '{'")?;
        let mut members = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            members.push(self.parse_member()?);
        }
        self.expect(TokenKind::RBrace, "expected '}'")?;
        Ok(InterfaceDecl {
            access,
            name,
            type_params,
            members,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_type_params(&mut self) -> CompileResult<Vec<String>> {
        if !self.match_kind(&[TokenKind::Lt]) {
            return Ok(vec![]);
        }
        let mut params = Vec::new();
        let (name, _) = self.expect_ident()?;
        params.push(name);
        while self.match_kind(&[TokenKind::Comma]) {
            let (name, _) = self.expect_ident()?;
            params.push(name);
        }
        self.expect(TokenKind::Gt, "expected '>'")?;
        Ok(params)
    }

    fn parse_where_clause(&mut self) -> CompileResult<Vec<GenericConstraint>> {
        let mut constraints = Vec::new();
        if !self.match_kind(&[TokenKind::Where]) {
            return Ok(constraints);
        }
        loop {
            let (type_param, _) = self.expect_ident()?;
            self.expect(TokenKind::Colon, "expected ':' in where clause")?;
            let mut bounds = Vec::new();
            // `new()` special case
            if self.check(&TokenKind::New) {
                let span = self.current().span;
                self.advance();
                self.expect(TokenKind::LParen, "expected '('")?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                bounds.push(TypeRef::named("new()", span));
            } else {
                bounds.push(self.parse_type_ref()?);
            }
            while self.check(&TokenKind::Comma) {
                // Peek: if comma is followed by Ident Colon, it's a new constraint
                let is_next_constraint = matches!(self.peek_kind(1), Some(TokenKind::Ident(_)))
                    && matches!(self.peek_kind(2), Some(TokenKind::Colon));
                if is_next_constraint {
                    break;
                }
                self.advance(); // consume comma
                if self.check(&TokenKind::New) {
                    let span = self.current().span;
                    self.advance();
                    self.expect(TokenKind::LParen, "expected '('")?;
                    self.expect(TokenKind::RParen, "expected ')'")?;
                    bounds.push(TypeRef::named("new()", span));
                } else {
                    bounds.push(self.parse_type_ref()?);
                }
            }
            constraints.push(GenericConstraint { type_param, bounds });
            if !self.check(&TokenKind::Comma) {
                break;
            }
            self.advance(); // consume comma
        }
        Ok(constraints)
    }

    fn parse_member(&mut self) -> CompileResult<Member> {
        let _attrs = self.parse_attributes()?;
        let access = self.parse_access();

        // Destructor ~new()
        if self.check(&TokenKind::Tilde) {
            self.advance();
            self.expect(TokenKind::New, "expected 'new' after '~'")?;
            self.expect(TokenKind::LParen, "expected '('")?;
            self.expect(TokenKind::RParen, "expected ')'")?;
            let body = self.parse_block()?;
            return Ok(Member::Destructor(DestructorDecl {
                body,
                span: self.previous().span,
            }));
        }

        // Constructor new(...)
        if self.check(&TokenKind::New) {
            let start = self.current().span;
            self.advance();
            self.expect(TokenKind::LParen, "expected '('")?;
            let params = self.parse_param_list()?;
            self.expect(TokenKind::RParen, "expected ')'")?;
            let mut base_args = Vec::new();
            if self.match_kind(&[TokenKind::Colon]) {
                self.expect(TokenKind::Base, "expected 'base'")?;
                self.expect(TokenKind::LParen, "expected '('")?;
                if !self.check(&TokenKind::RParen) {
                    loop {
                        base_args.push(self.parse_expression()?);
                        if !self.match_kind(&[TokenKind::Comma]) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RParen, "expected ')'")?;
            }
            let body = self.parse_block()?;
            return Ok(Member::Constructor(ConstructorDecl {
                params,
                base_args,
                body,
                span: start.merge(self.previous().span),
            }));
        }

        let is_static = self.match_kind(&[TokenKind::Static]);
        let is_virtual = self.match_kind(&[TokenKind::Virtual]);
        let is_override = self.match_kind(&[TokenKind::Override]);
        let is_abstract = self.match_kind(&[TokenKind::Abstract]);
        let is_async = self.match_kind(&[TokenKind::Async]);
        let is_unsafe = self.match_kind(&[TokenKind::Unsafe]);
        let is_const = self.match_kind(&[TokenKind::Const]);

        // property
        if self.check(&TokenKind::Property) {
            return Ok(Member::Property(self.parse_property(access, is_static)?));
        }

        // operator
        if self.check(&TokenKind::Operator) || self.is_type_start() && matches!(self.peek_kind(1), Some(TokenKind::Operator)) {
            // return_type operator+(...)
            if !self.check(&TokenKind::Operator) {
                let return_type = self.parse_type_ref()?;
                self.expect(TokenKind::Operator, "expected 'operator'")?;
                let op = self.parse_operator_symbol()?;
                self.expect(TokenKind::LParen, "expected '('")?;
                let params = self.parse_param_list()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                let body = self.parse_block()?;
                return Ok(Member::Operator(OperatorDecl {
                    op,
                    params,
                    return_type,
                    body,
                    span: self.previous().span,
                }));
            }
        }

        // Indexer: Type this[...] 
        // Method / field

        // Could be field `int x;` or `int x = 1;` or method `int Foo(...)`
        let ty = if self.check(&TokenKind::Void) || self.is_type_start() {
            Some(self.parse_type_ref()?)
        } else {
            None
        };

        // Indexer this
        if self.check(&TokenKind::This) && matches!(self.peek_kind(1), Some(TokenKind::LBracket)) {
            let return_ty = ty.ok_or_else(|| {
                CompileError::syntax("indexer requires type", self.current().span)
            })?;
            self.advance(); // this
            self.expect(TokenKind::LBracket, "expected '['")?;
            let params = self.parse_param_list()?;
            self.expect(TokenKind::RBracket, "expected ']'")?;
            let (getter, setter, _) = self.parse_property_body()?;
            return Ok(Member::Indexer(IndexerDecl {
                ty: return_ty,
                params,
                getter,
                setter,
                span: self.previous().span,
            }));
        }

        let (name, name_span) = self.expect_ident()?;

        if self.check(&TokenKind::LParen) || self.check(&TokenKind::Lt) {
            // Method
            let type_params = self.parse_type_params()?;
            self.expect(TokenKind::LParen, "expected '('")?;
            let params = self.parse_param_list()?;
            self.expect(TokenKind::RParen, "expected ')'")?;
            let constraints = self.parse_where_clause()?;
            let body = if self.check(&TokenKind::Arrow) {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(TokenKind::Semicolon, "expected ';'")?;
                Some(FunctionBody::Expr(Box::new(expr)))
            } else if self.check(&TokenKind::LBrace) {
                Some(FunctionBody::Block(self.parse_block()?))
            } else if self.match_kind(&[TokenKind::Semicolon]) {
                None // abstract
            } else {
                return Err(CompileError::syntax(
                    "expected method body",
                    self.current().span,
                ));
            };
            return Ok(Member::Method(FunctionDecl {
                access,
                is_async,
                is_unsafe,
                is_static,
                is_virtual,
                is_override,
                is_abstract,
                is_extension: params.first().map(|p| p.is_this).unwrap_or(false),
                return_type: ty.unwrap_or_else(|| TypeRef::void(name_span)),
                name,
                type_params,
                params,
                constraints,
                body,
                attributes: vec![],
                span: name_span.merge(self.previous().span),
            }));
        }

        // Field
        let init = if self.match_kind(&[TokenKind::Eq]) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        self.expect(TokenKind::Semicolon, "expected ';' after field")?;
        Ok(Member::Field(FieldDecl {
            access,
            is_static,
            is_const,
            ty,
            name,
            init,
            span: name_span.merge(self.previous().span),
        }))
    }

    fn parse_operator_symbol(&mut self) -> CompileResult<String> {
        let tok = self.current().clone();
        let op = match tok.kind {
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Percent => "%",
            TokenKind::EqEq => "==",
            TokenKind::BangEq => "!=",
            TokenKind::Lt => "<",
            TokenKind::Gt => ">",
            TokenKind::LtEq => "<=",
            TokenKind::GtEq => ">=",
            _ => {
                return Err(CompileError::syntax(
                    "expected operator symbol",
                    tok.span,
                ));
            }
        };
        self.advance();
        Ok(op.to_string())
    }

    fn parse_property(&mut self, access: Access, is_static: bool) -> CompileResult<PropertyDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Property, "expected 'property'")?;
        let (name, _) = self.expect_ident()?;
        self.expect(TokenKind::Colon, "expected ':'")?;
        let ty = self.parse_type_ref()?;
        let (getter, setter, auto) = self.parse_property_body()?;
        Ok(PropertyDecl {
            access,
            is_static,
            name,
            ty,
            getter,
            setter,
            auto,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_property_body(&mut self) -> CompileResult<(Option<Block>, Option<Block>, bool)> {
        self.expect(TokenKind::LBrace, "expected '{'")?;
        let mut getter = None;
        let mut setter = None;
        let mut auto = false;
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            if self.match_kind(&[TokenKind::Get]) {
                if self.match_kind(&[TokenKind::Semicolon]) {
                    auto = true;
                    getter = Some(Block {
                        stmts: vec![],
                        span: self.previous().span,
                    });
                } else {
                    getter = Some(self.parse_block()?);
                }
            } else if self.match_kind(&[TokenKind::Set]) {
                if self.match_kind(&[TokenKind::Semicolon]) {
                    auto = true;
                    setter = Some(Block {
                        stmts: vec![],
                        span: self.previous().span,
                    });
                } else {
                    setter = Some(self.parse_block()?);
                }
            } else {
                return Err(CompileError::syntax(
                    "expected get or set",
                    self.current().span,
                ));
            }
        }
        self.expect(TokenKind::RBrace, "expected '}'")?;
        Ok((getter, setter, auto))
    }

    fn parse_function(
        &mut self,
        access: Access,
        is_abstract: bool,
        attributes: Vec<Attribute>,
    ) -> CompileResult<FunctionDecl> {
        let start = self.current().span;
        let is_async = self.match_kind(&[TokenKind::Async]);
        let is_unsafe = self.match_kind(&[TokenKind::Unsafe]);
        let is_static = self.match_kind(&[TokenKind::Static]);
        let is_virtual = self.match_kind(&[TokenKind::Virtual]);
        let is_override = self.match_kind(&[TokenKind::Override]);

        let return_type = self.parse_type_ref()?;
        let (name, _) = self.expect_ident()?;
        let type_params = self.parse_type_params()?;
        self.expect(TokenKind::LParen, "expected '('")?;
        let params = self.parse_param_list()?;
        self.expect(TokenKind::RParen, "expected ')'")?;
        let constraints = self.parse_where_clause()?;

        let body = if self.check(&TokenKind::Arrow) {
            self.advance();
            let expr = self.parse_expression()?;
            self.expect(TokenKind::Semicolon, "expected ';'")?;
            Some(FunctionBody::Expr(Box::new(expr)))
        } else if self.check(&TokenKind::LBrace) {
            Some(FunctionBody::Block(self.parse_block()?))
        } else if self.match_kind(&[TokenKind::Semicolon]) {
            None
        } else {
            return Err(CompileError::syntax(
                "expected function body",
                self.current().span,
            ));
        };

        Ok(FunctionDecl {
            access,
            is_async,
            is_unsafe,
            is_static,
            is_virtual,
            is_override,
            is_abstract,
            is_extension: params.first().map(|p| p.is_this).unwrap_or(false),
            return_type,
            name,
            type_params,
            params,
            constraints,
            body,
            attributes,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_param_list(&mut self) -> CompileResult<Vec<Param>> {
        let mut params = Vec::new();
        if self.check(&TokenKind::RParen) || self.check(&TokenKind::RBracket) {
            return Ok(params);
        }
        loop {
            params.push(self.parse_param()?);
            if !self.match_kind(&[TokenKind::Comma]) {
                break;
            }
        }
        Ok(params)
    }

    fn parse_param(&mut self) -> CompileResult<Param> {
        let start = self.current().span;
        let is_params = self.match_kind(&[TokenKind::Params]);
        let is_this = self.match_kind(&[TokenKind::This]);

        // name: type   OR   type name (C-style for some contexts)
        // Spec uses name: type
        if self.check_ident()
            && matches!(self.peek_kind(1), Some(TokenKind::Colon))
        {
            let (name, _) = self.expect_ident()?;
            self.expect(TokenKind::Colon, "expected ':'")?;
            let ty = self.parse_type_ref()?;
            let default = if self.match_kind(&[TokenKind::Eq]) {
                Some(self.parse_expression()?)
            } else {
                None
            };
            return Ok(Param {
                is_params,
                is_this,
                name,
                ty,
                default,
                span: start.merge(self.previous().span),
            });
        }

        // Fallback: Type name
        let ty = self.parse_type_ref()?;
        let (name, _) = if self.check_ident() {
            self.expect_ident()?
        } else {
            ("_".into(), start)
        };
        let default = if self.match_kind(&[TokenKind::Eq]) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        Ok(Param {
            is_params,
            is_this,
            name,
            ty,
            default,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_const_decl(&mut self) -> CompileResult<ConstDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Const, "expected 'const'")?;
        let ty = self.parse_type_ref()?;
        let (name, _) = self.expect_ident()?;
        self.expect(TokenKind::Eq, "expected '='")?;
        let value = self.parse_expression()?;
        self.expect(TokenKind::Semicolon, "expected ';'")?;
        Ok(ConstDecl {
            ty,
            name,
            value,
            span: start.merge(self.previous().span),
        })
    }

    // ─── Types ───────────────────────────────────────────────

    fn is_type_start(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Ident(_)
                | TokenKind::Void
                | TokenKind::Bool
                | TokenKind::Byte
                | TokenKind::SByte
                | TokenKind::Short
                | TokenKind::UShort
                | TokenKind::Int
                | TokenKind::UInt
                | TokenKind::Long
                | TokenKind::ULong
                | TokenKind::Float
                | TokenKind::Double
                | TokenKind::Decimal
                | TokenKind::Char
                | TokenKind::String
                | TokenKind::Ptr
                | TokenKind::Dyn
                | TokenKind::Var
                | TokenKind::Volatile
        )
    }

    fn parse_type_ref(&mut self) -> CompileResult<TypeRef> {
        let start = self.current().span;
        let volatile = self.match_kind(&[TokenKind::Volatile]);
        let name = match &self.current().kind {
            TokenKind::Ident(s) => {
                let s = s.clone();
                self.advance();
                s
            }
            TokenKind::Void => {
                self.advance();
                "void".into()
            }
            TokenKind::Bool => {
                self.advance();
                "bool".into()
            }
            TokenKind::Byte => {
                self.advance();
                "byte".into()
            }
            TokenKind::SByte => {
                self.advance();
                "sbyte".into()
            }
            TokenKind::Short => {
                self.advance();
                "short".into()
            }
            TokenKind::UShort => {
                self.advance();
                "ushort".into()
            }
            TokenKind::Int => {
                self.advance();
                "int".into()
            }
            TokenKind::UInt => {
                self.advance();
                "uint".into()
            }
            TokenKind::Long => {
                self.advance();
                "long".into()
            }
            TokenKind::ULong => {
                self.advance();
                "ulong".into()
            }
            TokenKind::Float => {
                self.advance();
                "float".into()
            }
            TokenKind::Double => {
                self.advance();
                "double".into()
            }
            TokenKind::Decimal => {
                self.advance();
                "decimal".into()
            }
            TokenKind::Char => {
                self.advance();
                "char".into()
            }
            TokenKind::String => {
                self.advance();
                "string".into()
            }
            TokenKind::Ptr => {
                self.advance();
                "ptr".into()
            }
            TokenKind::Dyn => {
                self.advance();
                "dyn".into()
            }
            TokenKind::Var => {
                self.advance();
                "var".into()
            }
            _ => {
                return Err(CompileError::syntax(
                    format!("expected type, found {}", self.current().lexeme),
                    self.current().span,
                ));
            }
        };

        let mut args = Vec::new();
        if self.match_kind(&[TokenKind::Lt]) {
            loop {
                args.push(self.parse_type_ref()?);
                if !self.match_kind(&[TokenKind::Comma]) {
                    break;
                }
            }
            self.expect(TokenKind::Gt, "expected '>'")?;
        }

        let mut is_array = false;
        let mut array_dims = 0;
        while self.check(&TokenKind::LBracket) {
            // Could be [] or [,]
            self.advance();
            array_dims = 1;
            while self.match_kind(&[TokenKind::Comma]) {
                array_dims += 1;
            }
            self.expect(TokenKind::RBracket, "expected ']'")?;
            is_array = true;
        }

        let nullable = self.match_kind(&[TokenKind::Question]);

        Ok(TypeRef {
            name,
            args,
            nullable,
            is_array,
            array_dims,
            volatile,
            span: start.merge(self.previous().span),
        })
    }

    // ─── Statements ──────────────────────────────────────────

    fn parse_block(&mut self) -> CompileResult<Block> {
        let start = self.current().span;
        self.expect(TokenKind::LBrace, "expected '{'")?;
        let mut stmts = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            stmts.push(self.parse_statement()?);
        }
        self.expect(TokenKind::RBrace, "expected '}'")?;
        Ok(Block {
            stmts,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_statement(&mut self) -> CompileResult<Stmt> {
        match &self.current().kind {
            TokenKind::LBrace => Ok(Stmt::Block(self.parse_block()?)),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::Do => self.parse_do_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Foreach => self.parse_foreach(),
            TokenKind::Switch => self.parse_switch(),
            TokenKind::Match => self.parse_match(),
            TokenKind::Try => self.parse_try(),
            TokenKind::Throw => {
                let start = self.current().span;
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(TokenKind::Semicolon, "expected ';'")?;
                Ok(Stmt::Throw(expr, start.merge(self.previous().span)))
            }
            TokenKind::Return => {
                let start = self.current().span;
                self.advance();
                let expr = if self.check(&TokenKind::Semicolon) {
                    None
                } else {
                    Some(self.parse_expression()?)
                };
                self.expect(TokenKind::Semicolon, "expected ';'")?;
                Ok(Stmt::Return(expr, start.merge(self.previous().span)))
            }
            TokenKind::Break => {
                let span = self.current().span;
                self.advance();
                self.expect(TokenKind::Semicolon, "expected ';'")?;
                Ok(Stmt::Break(span))
            }
            TokenKind::Continue => {
                let span = self.current().span;
                self.advance();
                self.expect(TokenKind::Semicolon, "expected ';'")?;
                Ok(Stmt::Continue(span))
            }
            TokenKind::Using => self.parse_using(),
            TokenKind::Unsafe => {
                let start = self.current().span;
                self.advance();
                let body = self.parse_block()?;
                Ok(Stmt::Unsafe(body, start.merge(self.previous().span)))
            }
            TokenKind::Asm => self.parse_asm_stmt(),
            TokenKind::Const => Ok(Stmt::Const(self.parse_const_decl()?)),
            TokenKind::Var
            | TokenKind::Dyn
            | TokenKind::Stack
            | TokenKind::Owned => Ok(Stmt::Decl(self.parse_var_decl()?)),
            _ if self.is_type_start() && self.looks_like_declaration() => {
                Ok(Stmt::Decl(self.parse_var_decl()?))
            }
            _ => {
                let expr = self.parse_expression()?;
                self.expect(TokenKind::Semicolon, "expected ';' after expression")?;
                Ok(Stmt::Expr(expr))
            }
        }
    }

    fn looks_like_declaration(&self) -> bool {
        // type name ... where name is Ident and next is Ident or `[` for arrays already in type
        // Heuristic: Type Ident (=|;|,)
        // Skip type-like tokens then check Ident
        let mut i = 0;
        if matches!(self.peek_kind(i), Some(TokenKind::Volatile)) {
            i += 1;
        }
        // skip type name
        match self.peek_kind(i) {
            Some(TokenKind::Ident(_))
            | Some(TokenKind::Int)
            | Some(TokenKind::String)
            | Some(TokenKind::Bool)
            | Some(TokenKind::Void)
            | Some(TokenKind::Float)
            | Some(TokenKind::Double)
            | Some(TokenKind::Long)
            | Some(TokenKind::Byte)
            | Some(TokenKind::Char)
            | Some(TokenKind::Dyn)
            | Some(TokenKind::Short)
            | Some(TokenKind::UShort)
            | Some(TokenKind::UInt)
            | Some(TokenKind::ULong)
            | Some(TokenKind::SByte)
            | Some(TokenKind::Decimal)
            | Some(TokenKind::Ptr) => i += 1,
            _ => return false,
        }
        // optional <...>
        if matches!(self.peek_kind(i), Some(TokenKind::Lt)) {
            i += 1;
            let mut depth = 1;
            while depth > 0 {
                match self.peek_kind(i) {
                    Some(TokenKind::Lt) => depth += 1,
                    Some(TokenKind::Gt) => depth -= 1,
                    Some(TokenKind::Eof) | None => return false,
                    _ => {}
                }
                i += 1;
            }
        }
        // optional []
        while matches!(self.peek_kind(i), Some(TokenKind::LBracket)) {
            i += 1;
            while !matches!(self.peek_kind(i), Some(TokenKind::RBracket)) {
                if matches!(self.peek_kind(i), Some(TokenKind::Eof) | None) {
                    return false;
                }
                i += 1;
            }
            i += 1;
        }
        // optional ?
        if matches!(self.peek_kind(i), Some(TokenKind::Question)) {
            i += 1;
        }
        matches!(self.peek_kind(i), Some(TokenKind::Ident(_)))
    }

    fn parse_var_decl(&mut self) -> CompileResult<VarDecl> {
        let start = self.current().span;
        let kind = if self.match_kind(&[TokenKind::Var]) {
            VarKind::Var
        } else if self.match_kind(&[TokenKind::Dyn]) {
            VarKind::Dyn
        } else if self.match_kind(&[TokenKind::Stack]) {
            self.match_kind(&[TokenKind::Var]);
            VarKind::Stack
        } else if self.match_kind(&[TokenKind::Owned]) {
            self.match_kind(&[TokenKind::Var]);
            VarKind::Owned
        } else {
            VarKind::Typed
        };

        let ty = if kind == VarKind::Var || kind == VarKind::Dyn {
            None
        } else if kind == VarKind::Stack || kind == VarKind::Owned {
            // stack var x = ... OR stack Type x
            if self.check(&TokenKind::Var) {
                self.advance();
                None
            } else if self.is_type_start() && self.looks_like_declaration() {
                Some(self.parse_type_ref()?)
            } else {
                None
            }
        } else {
            Some(self.parse_type_ref()?)
        };

        let (name, _) = self.expect_ident()?;
        let init = if self.match_kind(&[TokenKind::Eq]) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        self.expect(TokenKind::Semicolon, "expected ';'")?;
        Ok(VarDecl {
            kind,
            ty,
            name,
            init,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_if(&mut self) -> CompileResult<Stmt> {
        let start = self.current().span;
        self.expect(TokenKind::If, "expected 'if'")?;
        self.expect(TokenKind::LParen, "expected '('")?;
        let cond = self.parse_expression()?;
        self.expect(TokenKind::RParen, "expected ')'")?;
        let then_block = self.parse_block()?;
        let else_branch = if self.match_kind(&[TokenKind::Else]) {
            if self.check(&TokenKind::If) {
                Some(ElseBranch::If(Box::new(self.parse_if()?)))
            } else {
                Some(ElseBranch::Block(self.parse_block()?))
            }
        } else {
            None
        };
        Ok(Stmt::If {
            cond,
            then_block,
            else_branch,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_while(&mut self) -> CompileResult<Stmt> {
        let start = self.current().span;
        self.expect(TokenKind::While, "expected 'while'")?;
        self.expect(TokenKind::LParen, "expected '('")?;
        let cond = self.parse_expression()?;
        self.expect(TokenKind::RParen, "expected ')'")?;
        let body = self.parse_block()?;
        Ok(Stmt::While {
            cond,
            body,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_do_while(&mut self) -> CompileResult<Stmt> {
        let start = self.current().span;
        self.expect(TokenKind::Do, "expected 'do'")?;
        let body = self.parse_block()?;
        self.expect(TokenKind::While, "expected 'while'")?;
        self.expect(TokenKind::LParen, "expected '('")?;
        let cond = self.parse_expression()?;
        self.expect(TokenKind::RParen, "expected ')'")?;
        self.expect(TokenKind::Semicolon, "expected ';'")?;
        Ok(Stmt::DoWhile {
            body,
            cond,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_for(&mut self) -> CompileResult<Stmt> {
        let start = self.current().span;
        self.expect(TokenKind::For, "expected 'for'")?;
        self.expect(TokenKind::LParen, "expected '('")?;
        let init = if self.check(&TokenKind::Semicolon) {
            None
        } else if self.check(&TokenKind::Var) || self.looks_like_declaration() {
            Some(Box::new(Stmt::Decl(self.parse_var_decl_no_semi()?)))
        } else {
            let expr = self.parse_expression()?;
            self.expect(TokenKind::Semicolon, "expected ';'")?;
            Some(Box::new(Stmt::Expr(expr)))
        };
        if init.is_none() {
            self.expect(TokenKind::Semicolon, "expected ';'")?;
        }
        // parse_var_decl includes semicolon — adjust
        // Actually I used parse_var_decl_no_semi for init... need to implement
        let cond = if self.check(&TokenKind::Semicolon) {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.expect(TokenKind::Semicolon, "expected ';'")?;
        let step = if self.check(&TokenKind::RParen) {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.expect(TokenKind::RParen, "expected ')'")?;
        let body = self.parse_block()?;
        Ok(Stmt::For {
            init,
            cond,
            step,
            body,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_var_decl_no_semi(&mut self) -> CompileResult<VarDecl> {
        let start = self.current().span;
        let kind = if self.match_kind(&[TokenKind::Var]) {
            VarKind::Var
        } else if self.match_kind(&[TokenKind::Dyn]) {
            VarKind::Dyn
        } else {
            VarKind::Typed
        };
        let ty = if kind == VarKind::Typed {
            Some(self.parse_type_ref()?)
        } else {
            None
        };
        let (name, _) = self.expect_ident()?;
        let init = if self.match_kind(&[TokenKind::Eq]) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        self.expect(TokenKind::Semicolon, "expected ';'")?;
        Ok(VarDecl {
            kind,
            ty,
            name,
            init,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_foreach(&mut self) -> CompileResult<Stmt> {
        let start = self.current().span;
        self.expect(TokenKind::Foreach, "expected 'foreach'")?;
        self.expect(TokenKind::LParen, "expected '('")?;
        self.match_kind(&[TokenKind::Var]);
        let (var_name, _) = self.expect_ident()?;
        let index_name = if self.match_kind(&[TokenKind::Comma]) {
            self.match_kind(&[TokenKind::Var]);
            Some(self.expect_ident()?.0)
        } else {
            None
        };
        self.expect(TokenKind::In, "expected 'in'")?;
        let iter = self.parse_expression()?;
        self.expect(TokenKind::RParen, "expected ')'")?;
        let body = self.parse_block()?;
        Ok(Stmt::Foreach {
            var_name,
            index_name,
            iter,
            body,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_switch(&mut self) -> CompileResult<Stmt> {
        let start = self.current().span;
        self.expect(TokenKind::Switch, "expected 'switch'")?;
        self.expect(TokenKind::LParen, "expected '('")?;
        let expr = self.parse_expression()?;
        self.expect(TokenKind::RParen, "expected ')'")?;
        self.expect(TokenKind::LBrace, "expected '{'")?;
        let mut cases = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            if self.match_kind(&[TokenKind::Case]) {
                // Parse one or more patterns separated by `|`
                let mut patterns = Vec::new();
                loop {
                    let pat_expr = self.parse_expression()?;
                    // Check for range `..` (two dots)
                    if self.check(&TokenKind::DotDot) {
                        self.advance();
                        let end_expr = self.parse_expression()?;
                        patterns.push(crate::ast::SwitchPattern::Range(pat_expr, end_expr));
                    } else {
                        patterns.push(crate::ast::SwitchPattern::Expr(pat_expr));
                    }
                    // Multi-pattern: `|`
                    if self.check(&TokenKind::Pipe) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                // Optional binding: `case 42 x:`  (bind matched value to `x`)
                let pattern_bind = if self.check_ident()
                    && !self.check_ident_str("when")
                    && !self.check(&TokenKind::Colon)
                    && !self.is_keyword_current()
                {
                    Some(self.expect_ident()?.0)
                } else {
                    None
                };
                // Optional guard: `when <expr>`
                let guard = if self.check_ident_str("when") {
                    self.advance(); // consume `when`
                    Some(self.parse_expression()?)
                } else {
                    None
                };
                self.expect(TokenKind::Colon, "expected ':'")?;
                let body = self.parse_switch_body()?;
                cases.push(SwitchCase {
                    patterns,
                    pattern_bind,
                    guard,
                    body,
                });
            } else if self.match_kind(&[TokenKind::Default]) {
                self.expect(TokenKind::Colon, "expected ':'")?;
                let body = self.parse_switch_body()?;
                cases.push(SwitchCase {
                    patterns: vec![],
                    pattern_bind: None,
                    guard: None,
                    body,
                });
            } else {
                return Err(CompileError::syntax(
                    "expected 'case' or 'default'",
                    self.current().span,
                ));
            }
        }
        self.expect(TokenKind::RBrace, "expected '}'")?;
        Ok(Stmt::Switch {
            expr,
            cases,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_switch_body(&mut self) -> CompileResult<Vec<Stmt>> {
        let mut body = Vec::new();
        while !matches!(
            self.current().kind,
            TokenKind::Case | TokenKind::Default | TokenKind::RBrace
        ) && !self.is_at_end()
        {
            body.push(self.parse_statement()?);
        }
        Ok(body)
    }

    fn check_ident_str(&self, s: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Ident(name) if name == s)
    }

    fn is_keyword_current(&self) -> bool {
        !matches!(self.current().kind, TokenKind::Ident(_))
    }

    fn parse_match(&mut self) -> CompileResult<Stmt> {
        let start = self.current().span;
        self.expect(TokenKind::Match, "expected 'match'")?;
        self.expect(TokenKind::LParen, "expected '('")?;
        let expr = self.parse_expression()?;
        self.expect(TokenKind::RParen, "expected ')'")?;
        self.expect(TokenKind::LBrace, "expected '{'")?;
        let mut arms = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let (pattern, _) = self.expect_ident()?;
            let bind = if self.match_kind(&[TokenKind::LParen]) {
                let (b, _) = self.expect_ident()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                Some(b)
            } else {
                None
            };
            self.expect(TokenKind::Arrow, "expected '=>'")?;
            let body = self.parse_expression()?;
            self.match_kind(&[TokenKind::Comma]);
            arms.push(MatchArm {
                pattern,
                bind,
                body,
            });
        }
        self.expect(TokenKind::RBrace, "expected '}'")?;
        Ok(Stmt::Match {
            expr,
            arms,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_try(&mut self) -> CompileResult<Stmt> {
        let start = self.current().span;
        self.expect(TokenKind::Try, "expected 'try'")?;
        let body = self.parse_block()?;
        let mut catches = Vec::new();
        while self.match_kind(&[TokenKind::Catch]) {
            self.expect(TokenKind::LParen, "expected '('")?;
            let exception_type = if self.check(&TokenKind::RParen) {
                None
            } else {
                Some(self.parse_type_ref()?)
            };
            let name = if self.check_ident() {
                Some(self.expect_ident()?.0)
            } else {
                None
            };
            self.expect(TokenKind::RParen, "expected ')'")?;
            let catch_body = self.parse_block()?;
            catches.push(CatchClause {
                exception_type,
                name,
                body: catch_body,
            });
        }
        let finally = if self.match_kind(&[TokenKind::Finally]) {
            Some(self.parse_block()?)
        } else {
            None
        };
        Ok(Stmt::Try {
            body,
            catches,
            finally,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_using(&mut self) -> CompileResult<Stmt> {
        let start = self.current().span;
        self.expect(TokenKind::Using, "expected 'using'")?;
        self.expect(TokenKind::LParen, "expected '('")?;
        // using (var file = ...)
        let decl = if self.check(&TokenKind::Var) || self.is_type_start() {
            // parse without requiring outer semicolon handling
            let kind = if self.match_kind(&[TokenKind::Var]) {
                VarKind::Var
            } else {
                VarKind::Typed
            };
            let ty = if kind == VarKind::Typed {
                Some(self.parse_type_ref()?)
            } else {
                None
            };
            let (name, _) = self.expect_ident()?;
            self.expect(TokenKind::Eq, "expected '='")?;
            let init = Some(self.parse_expression()?);
            VarDecl {
                kind,
                ty,
                name,
                init,
                span: start,
            }
        } else {
            return Err(CompileError::syntax(
                "expected declaration in using",
                self.current().span,
            ));
        };
        self.expect(TokenKind::RParen, "expected ')'")?;
        let body = self.parse_block()?;
        Ok(Stmt::Using {
            decl,
            body,
            span: start.merge(self.previous().span),
        })
    }

    // ─── Expressions ─────────────────────────────────────────

    pub fn parse_expression(&mut self) -> CompileResult<Expr> {
        self.parse_assignment()
    }

    fn parse_assignment(&mut self) -> CompileResult<Expr> {
        let expr = self.parse_ternary()?;
        if let Some(op) = self.match_assign_op() {
            let value = self.parse_assignment()?;
            let span = expr.span().merge(value.span());
            return Ok(Expr::Assign {
                target: Box::new(expr),
                op,
                value: Box::new(value),
                span,
            });
        }
        Ok(expr)
    }

    fn match_assign_op(&mut self) -> Option<AssignOp> {
        let op = match self.current().kind {
            TokenKind::Eq => AssignOp::Assign,
            TokenKind::PlusEq => AssignOp::Add,
            TokenKind::MinusEq => AssignOp::Sub,
            TokenKind::StarEq => AssignOp::Mul,
            TokenKind::SlashEq => AssignOp::Div,
            TokenKind::PercentEq => AssignOp::Mod,
            TokenKind::AmpEq => AssignOp::BitAnd,
            TokenKind::PipeEq => AssignOp::BitOr,
            TokenKind::CaretEq => AssignOp::BitXor,
            TokenKind::LtLtEq => AssignOp::Shl,
            TokenKind::GtGtEq => AssignOp::Shr,
            TokenKind::QuestionQuestionEq => AssignOp::NullCoalesce,
            _ => return None,
        };
        self.advance();
        Some(op)
    }

    fn parse_ternary(&mut self) -> CompileResult<Expr> {
        let cond = self.parse_null_coalesce()?;
        if self.match_kind(&[TokenKind::Question]) {
            // Ambiguity with nullable — only if not followed by things that look like type continuation
            // In expression context after null coalesce, `?` starts ternary
            let then_expr = self.parse_expression()?;
            self.expect(TokenKind::Colon, "expected ':' in ternary")?;
            let else_expr = self.parse_ternary()?;
            let span = cond.span().merge(else_expr.span());
            return Ok(Expr::Ternary {
                cond: Box::new(cond),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
                span,
            });
        }
        Ok(cond)
    }

    fn parse_null_coalesce(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_or()?;
        while self.match_kind(&[TokenKind::QuestionQuestion]) {
            let right = self.parse_or()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::NullCoalesce,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_or(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_and()?;
        while self.match_kind(&[TokenKind::PipePipe]) {
            let right = self.parse_and()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::Or,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_bit_or()?;
        while self.match_kind(&[TokenKind::AmpAmp]) {
            let right = self.parse_bit_or()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::And,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_bit_or(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_bit_xor()?;
        while self.check(&TokenKind::Pipe) {
            self.advance();
            let right = self.parse_bit_xor()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::BitOr,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_bit_xor(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_bit_and()?;
        while self.match_kind(&[TokenKind::Caret]) {
            let right = self.parse_bit_and()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::BitXor,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_bit_and(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_equality()?;
        while self.check(&TokenKind::Amp) {
            self.advance();
            let right = self.parse_equality()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::BitAnd,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = if self.match_kind(&[TokenKind::EqEq]) {
                BinOp::Eq
            } else if self.match_kind(&[TokenKind::BangEq]) {
                BinOp::Ne
            } else {
                break;
            };
            let right = self.parse_comparison()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_shift()?;
        loop {
            // Avoid consuming `>` of generics — in expr context OK
            let op = if self.match_kind(&[TokenKind::LtEq]) {
                BinOp::Le
            } else if self.match_kind(&[TokenKind::GtEq]) {
                BinOp::Ge
            } else if self.check(&TokenKind::Lt) {
                self.advance();
                BinOp::Lt
            } else if self.check(&TokenKind::Gt) {
                self.advance();
                BinOp::Gt
            } else {
                break;
            };
            let right = self.parse_shift()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            };
        }
        // is / as
        if self.match_kind(&[TokenKind::Is]) {
            let ty = self.parse_type_ref()?;
            let span = left.span().merge(ty.span);
            left = Expr::Is {
                expr: Box::new(left),
                ty,
                span,
            };
        } else if self.match_kind(&[TokenKind::As]) {
            let ty = self.parse_type_ref()?;
            let span = left.span().merge(ty.span);
            left = Expr::As {
                expr: Box::new(left),
                ty,
                span,
            };
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_term()?;
        loop {
            let op = if self.match_kind(&[TokenKind::LtLt]) {
                BinOp::Shl
            } else if self.match_kind(&[TokenKind::GtGt]) {
                BinOp::Shr
            } else {
                break;
            };
            let right = self.parse_term()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_factor()?;
        loop {
            let op = if self.match_kind(&[TokenKind::Plus]) {
                BinOp::Add
            } else if self.match_kind(&[TokenKind::Minus]) {
                BinOp::Sub
            } else {
                break;
            };
            let right = self.parse_factor()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> CompileResult<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let op = if self.match_kind(&[TokenKind::Star]) {
                BinOp::Mul
            } else if self.match_kind(&[TokenKind::Slash]) {
                BinOp::Div
            } else if self.match_kind(&[TokenKind::Percent]) {
                BinOp::Mod
            } else {
                break;
            };
            let right = self.parse_unary()?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> CompileResult<Expr> {
        let start = self.current().span;
        if self.match_kind(&[TokenKind::Bang]) {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnOp::Not,
                span: start.merge(expr.span()),
                expr: Box::new(expr),
            });
        }
        if self.match_kind(&[TokenKind::Minus]) {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnOp::Neg,
                span: start.merge(expr.span()),
                expr: Box::new(expr),
            });
        }
        if self.match_kind(&[TokenKind::Tilde]) {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnOp::BitNot,
                span: start.merge(expr.span()),
                expr: Box::new(expr),
            });
        }
        if self.match_kind(&[TokenKind::PlusPlus]) {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnOp::PreInc,
                span: start.merge(expr.span()),
                expr: Box::new(expr),
            });
        }
        if self.match_kind(&[TokenKind::MinusMinus]) {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnOp::PreDec,
                span: start.merge(expr.span()),
                expr: Box::new(expr),
            });
        }
        if self.match_kind(&[TokenKind::Star]) {
            let expr = self.parse_unary()?;
            return Ok(Expr::Deref(Box::new(expr), start.merge(self.previous().span)));
        }
        if self.match_kind(&[TokenKind::Amp]) {
            let expr = self.parse_unary()?;
            return Ok(Expr::AddressOf(
                Box::new(expr),
                start.merge(self.previous().span),
            ));
        }
        if self.match_kind(&[TokenKind::Await]) {
            let expr = self.parse_unary()?;
            let span = start.merge(expr.span());
            return Ok(Expr::Await(Box::new(expr), span));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> CompileResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.match_kind(&[TokenKind::LParen]) {
                let args = self.parse_arg_list()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                let span = expr.span().merge(self.previous().span);
                expr = Expr::Call {
                    callee: Box::new(expr),
                    type_args: vec![],
                    args,
                    span,
                };
            } else if self.check(&TokenKind::Lt) && self.looks_like_type_args() {
                // Generic call: foo<int>(...)
                let type_args = self.parse_type_args()?;
                self.expect(TokenKind::LParen, "expected '(' after type args")?;
                let args = self.parse_arg_list()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                let span = expr.span().merge(self.previous().span);
                expr = Expr::Call {
                    callee: Box::new(expr),
                    type_args,
                    args,
                    span,
                };
            } else if self.match_kind(&[TokenKind::LBracket]) {
                let mut indices = Vec::new();
                loop {
                    indices.push(self.parse_expression()?);
                    if !self.match_kind(&[TokenKind::Comma]) {
                        break;
                    }
                }
                self.expect(TokenKind::RBracket, "expected ']'")?;
                let span = expr.span().merge(self.previous().span);
                expr = Expr::Index {
                    object: Box::new(expr),
                    indices,
                    span,
                };
            } else if self.match_kind(&[TokenKind::Dot]) {
                let (field, _) = self.expect_member_name()?;
                let span = expr.span().merge(self.previous().span);
                expr = Expr::Member {
                    object: Box::new(expr),
                    field,
                    null_safe: false,
                    span,
                };
            } else if self.match_kind(&[TokenKind::QuestionDot]) {
                let (field, _) = self.expect_member_name()?;
                let span = expr.span().merge(self.previous().span);
                expr = Expr::Member {
                    object: Box::new(expr),
                    field,
                    null_safe: true,
                    span,
                };
            } else if self.match_kind(&[TokenKind::ThinArrow]) {
                let (field, _) = self.expect_member_name()?;
                let span = expr.span().merge(self.previous().span);
                expr = Expr::PtrMember {
                    object: Box::new(expr),
                    field,
                    span,
                };
            } else if self.match_kind(&[TokenKind::PlusPlus]) {
                let span = expr.span().merge(self.previous().span);
                expr = Expr::Unary {
                    op: UnOp::PostInc,
                    expr: Box::new(expr),
                    span,
                };
            } else if self.match_kind(&[TokenKind::MinusMinus]) {
                let span = expr.span().merge(self.previous().span);
                expr = Expr::Unary {
                    op: UnOp::PostDec,
                    expr: Box::new(expr),
                    span,
                };
            } else if self.check(&TokenKind::Question)
                && !matches!(
                    self.peek_kind(1),
                    Some(TokenKind::Ident(_))
                        | Some(TokenKind::IntLit(_))
                        | Some(TokenKind::StringLit(_))
                        | Some(TokenKind::LParen)
                        | Some(TokenKind::BoolLit(_))
                        | Some(TokenKind::FloatLit(_))
                        | Some(TokenKind::CharLit(_))
                        | Some(TokenKind::Minus)
                        | Some(TokenKind::Bang)
                        | Some(TokenKind::Tilde)
                )
            {
                // Postfix try `expr?` only when `?` is NOT starting a ternary.
                // Important: do not consume `?` when the next token looks like a ternary arm
                // (otherwise `cond ? 1 : 0` loses the `?` and fails to parse).
                self.advance();
                let span = expr.span().merge(self.previous().span);
                expr = Expr::Try(Box::new(expr), span);
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn expect_member_name(&mut self) -> CompileResult<(String, Span)> {
        // Allow keywords as member names in limited cases
        if self.check_ident() {
            return self.expect_ident();
        }
        // Length etc. might be Ident. Keywords like `new` no.
        Err(CompileError::syntax(
            "expected member name",
            self.current().span,
        ))
    }

    fn looks_like_type_args(&self) -> bool {
        // Heuristic: <Type> or <Type, Type> followed by (
        if !matches!(self.peek_kind(0), Some(TokenKind::Lt)) {
            return false;
        }
        let mut i = 1;
        let mut depth = 1;
        while depth > 0 {
            match self.peek_kind(i) {
                Some(TokenKind::Lt) => depth += 1,
                Some(TokenKind::Gt) => depth -= 1,
                Some(TokenKind::Eof) | None => return false,
                Some(TokenKind::Semicolon) | Some(TokenKind::LBrace) => return false,
                _ => {}
            }
            i += 1;
            if i > 32 {
                return false;
            }
        }
        matches!(self.peek_kind(i), Some(TokenKind::LParen))
    }

    fn parse_type_args(&mut self) -> CompileResult<Vec<TypeRef>> {
        self.expect(TokenKind::Lt, "expected '<'")?;
        let mut args = Vec::new();
        loop {
            args.push(self.parse_type_ref()?);
            if !self.match_kind(&[TokenKind::Comma]) {
                break;
            }
        }
        self.expect(TokenKind::Gt, "expected '>'")?;
        Ok(args)
    }

    fn parse_arg_list(&mut self) -> CompileResult<Vec<Arg>> {
        let mut args = Vec::new();
        if self.check(&TokenKind::RParen) {
            return Ok(args);
        }
        loop {
            let name = if self.check_ident()
                && matches!(self.peek_kind(1), Some(TokenKind::Colon))
                && !matches!(self.peek_kind(2), Some(TokenKind::Colon))
            {
                // named arg name: value — but type annotations also use :
                // Named args: Connect(port: 9090) — Ident Colon Expr
                // Ambiguous with nothing else. Use named if Colon not followed by type-looking for cast.
                let (n, _) = self.expect_ident()?;
                self.advance(); // :
                Some(n)
            } else {
                None
            };
            let value = self.parse_expression()?;
            args.push(Arg { name, value });
            if !self.match_kind(&[TokenKind::Comma]) {
                break;
            }
        }
        Ok(args)
    }

    fn parse_primary(&mut self) -> CompileResult<Expr> {
        let span = self.current().span;
        match &self.current().kind.clone() {
            TokenKind::IntLit(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Int(n, span))
            }
            TokenKind::UIntLit(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::UInt(n, span))
            }
            TokenKind::FloatLit(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Float(n, span))
            }
            TokenKind::DecimalLit(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expr::Decimal(s, span))
            }
            TokenKind::BoolLit(b) => {
                let b = *b;
                self.advance();
                Ok(Expr::Bool(b, span))
            }
            TokenKind::CharLit(c) => {
                let c = *c;
                self.advance();
                Ok(Expr::Char(c, span))
            }
            TokenKind::StringLit(s) => {
                let s = s.clone();
                self.advance();
                if s.starts_with('\u{0001}') {
                    // interpolated
                    Ok(self.parse_interpolated(&s[1..], span)?)
                } else {
                    Ok(Expr::String(s, span))
                }
            }
            TokenKind::RawStringLit(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expr::String(s, span))
            }
            TokenKind::Null => {
                self.advance();
                Ok(Expr::Null(span))
            }
            TokenKind::This => {
                self.advance();
                Ok(Expr::This(span))
            }
            TokenKind::Base => {
                self.advance();
                Ok(Expr::Base(span))
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                // Lambda: ( already handled; bare ident
                Ok(Expr::Ident(name, span))
            }
            TokenKind::New => self.parse_new(),
            TokenKind::Typeof => {
                self.advance();
                self.expect(TokenKind::LParen, "expected '('")?;
                let ty = self.parse_type_ref()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                Ok(Expr::TypeOf(ty, span.merge(self.previous().span)))
            }
            TokenKind::Nameof => {
                self.advance();
                self.expect(TokenKind::LParen, "expected '('")?;
                let target = self.parse_nameof_target()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                Ok(Expr::NameOf(target, span.merge(self.previous().span)))
            }
            TokenKind::Sizeof => {
                self.advance();
                self.expect(TokenKind::LParen, "expected '('")?;
                let ty = self.parse_type_ref()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                Ok(Expr::SizeOf(ty, span.merge(self.previous().span)))
            }
            TokenKind::Offsetof => {
                self.advance();
                self.expect(TokenKind::LParen, "expected '('")?;
                let ty = self.parse_type_ref()?;
                self.expect(TokenKind::Comma, "expected ','")?;
                let (field, _) = self.expect_ident()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                Ok(Expr::OffsetOf {
                    ty,
                    field,
                    span: span.merge(self.previous().span),
                })
            }
            TokenKind::LParen => {
                self.advance();
                // Could be grouped expr, cast (type)expr, or lambda (a: int) =>
                if self.looks_like_lambda() {
                    return self.parse_lambda_after_lparen(span);
                }
                // Cast: (Type) expr
                if self.is_type_start() && self.looks_like_cast() {
                    let ty = self.parse_type_ref()?;
                    self.expect(TokenKind::RParen, "expected ')'")?;
                    let expr = self.parse_unary()?;
                    let span = span.merge(expr.span());
                    return Ok(Expr::Cast {
                        ty,
                        expr: Box::new(expr),
                        span,
                    });
                }
                let expr = self.parse_expression()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                Ok(Expr::Grouped(Box::new(expr), span.merge(self.previous().span)))
            }
            TokenKind::LBracket => {
                // Array literal [1, 2, 3]
                self.advance();
                let mut elems = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    loop {
                        elems.push(self.parse_expression()?);
                        if !self.match_kind(&[TokenKind::Comma]) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RBracket, "expected ']'")?;
                Ok(Expr::ArrayLit(elems, span.merge(self.previous().span)))
            }
            // Type keywords used as Ident in expressions like int.Parse
            TokenKind::Int
            | TokenKind::String
            | TokenKind::Bool
            | TokenKind::Float
            | TokenKind::Double
            | TokenKind::Long
            | TokenKind::Byte
            | TokenKind::Char
            | TokenKind::Void
            | TokenKind::Ptr
            | TokenKind::UInt
            | TokenKind::Short
            | TokenKind::UShort
            | TokenKind::SByte
            | TokenKind::ULong
            | TokenKind::Decimal
            | TokenKind::Dyn => {
                let name = self.current().lexeme.clone();
                self.advance();
                Ok(Expr::Ident(name, span))
            }
            _ => Err(CompileError::syntax(
                format!("unexpected token '{}'", self.current().lexeme),
                span,
            )),
        }
    }

    fn looks_like_lambda(&self) -> bool {
        // () =>  or (a: type) =>  or (a) =>  or (a, b) =>
        if self.check(&TokenKind::RParen) {
            return matches!(
                self.peek_kind(1),
                Some(TokenKind::Arrow) | Some(TokenKind::LBrace)
            );
        }
        if self.check_ident() {
            match self.peek_kind(1) {
                Some(TokenKind::Colon) => return true, // (a: int) =>
                Some(TokenKind::RParen) => {
                    return matches!(
                        self.peek_kind(2),
                        Some(TokenKind::Arrow) | Some(TokenKind::LBrace)
                    );
                }
                Some(TokenKind::Comma) => return true, // (a, b) =>
                _ => {}
            }
        }
        false
    }

    fn looks_like_cast(&self) -> bool {
        // (Type) primary — Type then )
        let mut i = 0;
        match self.peek_kind(i) {
            Some(TokenKind::Ident(_))
            | Some(TokenKind::Int)
            | Some(TokenKind::String)
            | Some(TokenKind::Bool)
            | Some(TokenKind::Float)
            | Some(TokenKind::Double)
            | Some(TokenKind::Long)
            | Some(TokenKind::Byte)
            | Some(TokenKind::Char)
            | Some(TokenKind::Ptr)
            | Some(TokenKind::Dyn) => i += 1,
            _ => return false,
        }
        if matches!(self.peek_kind(i), Some(TokenKind::Lt)) {
            i += 1;
            let mut depth = 1;
            while depth > 0 {
                match self.peek_kind(i) {
                    Some(TokenKind::Lt) => depth += 1,
                    Some(TokenKind::Gt) => depth -= 1,
                    Some(TokenKind::Eof) | None => return false,
                    _ => {}
                }
                i += 1;
            }
        }
        matches!(self.peek_kind(i), Some(TokenKind::RParen))
            && matches!(
                self.peek_kind(i + 1),
                Some(TokenKind::Ident(_))
                    | Some(TokenKind::LParen)
                    | Some(TokenKind::IntLit(_))
                    | Some(TokenKind::StringLit(_))
                    | Some(TokenKind::This)
            )
    }

    fn parse_lambda_after_lparen(&mut self, start: Span) -> CompileResult<Expr> {
        let params = if self.check(&TokenKind::RParen) {
            vec![]
        } else {
            self.parse_lambda_param_list()?
        };
        self.expect(TokenKind::RParen, "expected ')'")?;
        let body = if self.match_kind(&[TokenKind::Arrow]) {
            FunctionBody::Expr(Box::new(self.parse_expression()?))
        } else {
            FunctionBody::Block(self.parse_block()?)
        };
        Ok(Expr::Lambda {
            params,
            body,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_lambda_param_list(&mut self) -> CompileResult<Vec<Param>> {
        let mut params = Vec::new();
        loop {
            if self.check_ident() && !matches!(self.peek_kind(1), Some(TokenKind::Colon)) {
                let (name, span) = self.expect_ident()?;
                params.push(Param {
                    is_params: false,
                    is_this: false,
                    name,
                    ty: TypeRef::named("dyn", span),
                    default: None,
                    span,
                });
            } else {
                params.push(self.parse_param()?);
            }
            if !self.match_kind(&[TokenKind::Comma]) {
                break;
            }
        }
        Ok(params)
    }

    fn parse_asm_stmt(&mut self) -> CompileResult<Stmt> {
        let start = self.current().span;
        self.expect(TokenKind::Asm, "expected 'asm'")?;
        // Optional `volatile` (C: `asm volatile (...)`); we always emit volatile.
        let _explicit_volatile = self.match_kind(&[TokenKind::Volatile]);
        self.expect(TokenKind::LParen, "expected '(' after asm")?;

        let mut template = self.parse_asm_template()?;
        let mut outputs = Vec::new();
        let mut inputs = Vec::new();
        let mut clobbers = Vec::new();

        if self.match_kind(&[TokenKind::Colon]) {
            // GCC-style: asm("..." : outs : ins : clobbers)
            if !self.check(&TokenKind::Colon) && !self.check(&TokenKind::RParen) {
                outputs = self.parse_asm_operand_list()?;
            }
            if self.match_kind(&[TokenKind::Colon]) {
                if !self.check(&TokenKind::Colon) && !self.check(&TokenKind::RParen) {
                    inputs = self.parse_asm_operand_list()?;
                }
                if self.match_kind(&[TokenKind::Colon]) {
                    if !self.check(&TokenKind::RParen) {
                        clobbers = self.parse_asm_clobber_list()?;
                    }
                }
            }
        } else {
            // Sugar: asm("...", out x, in y) or asm("...", "=r"(x), "r"(y))
            while self.match_kind(&[TokenKind::Comma]) {
                let (op, is_out) = self.parse_asm_sugar_operand()?;
                if is_out {
                    outputs.push(op);
                } else {
                    inputs.push(op);
                }
            }
            // Convert `{N}` placeholders to GCC `%N`
            if template.contains('{') {
                template = rewrite_asm_braces(&template);
            }
        }

        self.expect(TokenKind::RParen, "expected ')' after asm")?;
        self.expect(TokenKind::Semicolon, "expected ';' after asm")?;
        Ok(Stmt::Asm {
            template,
            outputs,
            inputs,
            clobbers,
            is_volatile: true,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_asm_template(&mut self) -> CompileResult<String> {
        let mut parts = Vec::new();
        loop {
            match &self.current().kind {
                TokenKind::StringLit(s) | TokenKind::RawStringLit(s) => {
                    parts.push(s.clone());
                    self.advance();
                }
                _ => break,
            }
        }
        if parts.is_empty() {
            return Err(CompileError::syntax(
                "expected string literal in asm(...)",
                self.current().span,
            ));
        }
        Ok(parts.join(""))
    }

    fn parse_asm_operand_list(&mut self) -> CompileResult<Vec<AsmOperand>> {
        let mut ops = Vec::new();
        loop {
            ops.push(self.parse_asm_c_operand()?);
            if !self.match_kind(&[TokenKind::Comma]) {
                break;
            }
            // Trailing comma before `:)` / `)` is not allowed in C; stop if next is colon/rparen
            if self.check(&TokenKind::Colon) || self.check(&TokenKind::RParen) {
                break;
            }
        }
        Ok(ops)
    }

    /// C-style `"=r"(expr)`.
    fn parse_asm_c_operand(&mut self) -> CompileResult<AsmOperand> {
        let constraint = match &self.current().kind {
            TokenKind::StringLit(s) | TokenKind::RawStringLit(s) => {
                let s = s.clone();
                self.advance();
                s
            }
            _ => {
                return Err(CompileError::syntax(
                    "expected constraint string in asm operand (e.g. \"=r\"(x))",
                    self.current().span,
                ));
            }
        };
        self.expect(TokenKind::LParen, "expected '(' after asm constraint")?;
        let expr = self.parse_expression()?;
        self.expect(TokenKind::RParen, "expected ')' after asm operand")?;
        Ok(AsmOperand { constraint, expr })
    }

    /// Sugar `out x` / `in y` / `out "=r" x` / `"=r"(x)`.
    /// Note: `in` is a language keyword (`TokenKind::In`), not an ident.
    fn parse_asm_sugar_operand(&mut self) -> CompileResult<(AsmOperand, bool)> {
        // Direct C operand after comma
        if matches!(
            self.current().kind,
            TokenKind::StringLit(_) | TokenKind::RawStringLit(_)
        ) {
            let op = self.parse_asm_c_operand()?;
            let is_out = op.constraint.starts_with('=') || op.constraint.starts_with('+');
            return Ok((op, is_out));
        }
        let is_out = if self.match_kind(&[TokenKind::In]) {
            false
        } else {
            let (kw, _) = self.expect_ident()?;
            match kw.as_str() {
                "out" | "output" => true,
                "input" => false,
                other => {
                    return Err(CompileError::syntax(
                        format!("expected 'out' or 'in' in asm sugar, found '{other}'"),
                        self.previous().span,
                    ));
                }
            }
        };
        let constraint = if matches!(
            self.current().kind,
            TokenKind::StringLit(_) | TokenKind::RawStringLit(_)
        ) {
            match &self.current().kind {
                TokenKind::StringLit(s) | TokenKind::RawStringLit(s) => {
                    let s = s.clone();
                    self.advance();
                    s
                }
                _ => unreachable!(),
            }
        } else if is_out {
            "=r".into()
        } else {
            "r".into()
        };
        let expr = self.parse_expression()?;
        Ok((AsmOperand { constraint, expr }, is_out))
    }

    fn parse_asm_clobber_list(&mut self) -> CompileResult<Vec<String>> {
        let mut out = Vec::new();
        loop {
            match &self.current().kind {
                TokenKind::StringLit(s) | TokenKind::RawStringLit(s) => {
                    out.push(s.clone());
                    self.advance();
                }
                _ => {
                    return Err(CompileError::syntax(
                        "expected clobber string (e.g. \"memory\")",
                        self.current().span,
                    ));
                }
            }
            if !self.match_kind(&[TokenKind::Comma]) {
                break;
            }
            if self.check(&TokenKind::RParen) {
                break;
            }
        }
        Ok(out)
    }

    fn parse_nameof_target(&mut self) -> CompileResult<NameOfExpr> {
        let first = if self.match_kind(&[TokenKind::This]) {
            "this".to_string()
        } else {
            self.expect_ident()?.0
        };
        if !self.check(&TokenKind::Dot) {
            return Ok(NameOfExpr::Ident(first));
        }
        let mut object = Expr::Ident(first, self.previous().span);
        let mut field = String::new();
        while self.match_kind(&[TokenKind::Dot]) {
            let (name, fspan) = self.expect_ident()?;
            field = name.clone();
            let span = object.span().merge(fspan);
            object = Expr::Member {
                object: Box::new(object),
                field: name,
                null_safe: false,
                span,
            };
        }
        Ok(NameOfExpr::Member {
            object: Box::new(object),
            field,
        })
    }

    fn parse_new(&mut self) -> CompileResult<Expr> {
        let start = self.current().span;
        self.expect(TokenKind::New, "expected 'new'")?;
        let ty = self.parse_type_ref()?;
        let mut args = Vec::new();
        if self.match_kind(&[TokenKind::LParen]) {
            args = self.parse_arg_list()?;
            self.expect(TokenKind::RParen, "expected ')'")?;
        } else if ty.is_array && self.check(&TokenKind::LBrace) {
            // new T[] { ... } — array already in type; also new int[10]
        }

        // new int[10] — type parse may have consumed [ as array type without size
        // Handle object initializer
        let mut init = Vec::new();
        if self.match_kind(&[TokenKind::LBrace]) {
            if !self.check(&TokenKind::RBrace) {
                // Could be collection initializer or object initializer
                if self.check_ident() && matches!(self.peek_kind(1), Some(TokenKind::Eq)) {
                    loop {
                        let (name, _) = self.expect_ident()?;
                        self.expect(TokenKind::Eq, "expected '='")?;
                        let value = self.parse_expression()?;
                        init.push((name, value));
                        if !self.match_kind(&[TokenKind::Comma]) {
                            break;
                        }
                    }
                } else {
                    // collection { a, b, c }
                    loop {
                        let value = self.parse_expression()?;
                        init.push((String::new(), value));
                        if !self.match_kind(&[TokenKind::Comma]) {
                            break;
                        }
                        if self.check(&TokenKind::RBrace) {
                            break;
                        }
                    }
                }
            }
            self.expect(TokenKind::RBrace, "expected '}'")?;
        }

        Ok(Expr::New {
            ty,
            args,
            init,
            span: start.merge(self.previous().span),
        })
    }

    fn parse_interpolated(&self, s: &str, span: Span) -> CompileResult<Expr> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '{' {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    literal.push('{');
                    continue;
                }
                if !literal.is_empty() {
                    parts.push(InterpPart::Literal(std::mem::take(&mut literal)));
                }
                let mut expr_src = String::new();
                let mut depth = 1;
                for c2 in chars.by_ref() {
                    if c2 == '{' {
                        depth += 1;
                        expr_src.push(c2);
                    } else if c2 == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        expr_src.push(c2);
                    } else {
                        expr_src.push(c2);
                    }
                }
                // Parse expression from expr_src
                let tokens = crate::lexer::Lexer::new(&expr_src).tokenize()?;
                let mut parser = Parser::new(tokens);
                let expr = parser.parse_expression()?;
                parts.push(InterpPart::Expr(expr));
            } else if c == '}' {
                if chars.peek() == Some(&'}') {
                    chars.next();
                    literal.push('}');
                } else {
                    literal.push(c);
                }
            } else {
                literal.push(c);
            }
        }
        if !literal.is_empty() {
            parts.push(InterpPart::Literal(literal));
        }
        Ok(Expr::Interpolated(parts, span))
    }
}

fn apply_layout_attributes(item: Item, attrs: &[Attribute]) -> Item {
    match item {
        Item::Struct(mut s) => {
            for a in attrs {
                match a.name.as_str() {
                    "packed" => s.packed = true,
                    "align" => {
                        if let Some(Expr::Int(n, _)) = &a.value {
                            s.align = Some(*n as u32);
                        }
                    }
                    "repr" => {
                        if let Some(Expr::String(v, _)) = &a.value {
                            if v.eq_ignore_ascii_case("c") {
                                s.repr_c = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            s.attributes.extend(attrs.iter().cloned());
            Item::Struct(s)
        }
        Item::Union(mut u) => {
            for a in attrs {
                match a.name.as_str() {
                    "packed" => u.packed = true,
                    "align" => {
                        if let Some(Expr::Int(n, _)) = &a.value {
                            u.align = Some(*n as u32);
                        }
                    }
                    _ => {}
                }
            }
            u.attributes.extend(attrs.iter().cloned());
            Item::Union(u)
        }
        other => other,
    }
}

/// Rewrite RayTask `{0}` / `{1}` asm placeholders to GCC `%0` / `%1`.
fn rewrite_asm_braces(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start && j < bytes.len() && bytes[j] == b'}' {
                out.push('%');
                out.push_str(&template[start..j]);
                i = j + 1;
                continue;
            }
        }
        // Escape lone `%` for GCC? leave as-is so C `%eax` / `%0` still work.
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
