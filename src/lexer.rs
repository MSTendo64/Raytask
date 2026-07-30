//! Lexical analyzer for RayTask (.rt) source files.

use crate::error::{CompileError, CompileResult};
use crate::span::Span;
use crate::token::{keyword, Token, TokenKind};

pub struct Lexer<'a> {
    source: &'a str,
    chars: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn tokenize(mut self) -> CompileResult<Vec<Token>> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = tok.is_eof();
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn span_start(&self) -> (usize, usize, usize) {
        (self.pos, self.line, self.column)
    }

    fn make_span(&self, start: usize, line: usize, column: usize) -> Span {
        Span::new(start, self.pos, line, column)
    }

    fn skip_whitespace_and_comments(&mut self) -> CompileResult<()> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.advance();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    // // or ///
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    let (start, line, col) = self.span_start();
                    self.advance(); // /
                    self.advance(); // *
                    let mut closed = false;
                    while let Some(c) = self.peek() {
                        if c == '*' && self.peek_at(1) == Some('/') {
                            self.advance();
                            self.advance();
                            closed = true;
                            break;
                        }
                        self.advance();
                    }
                    if !closed {
                        return Err(CompileError::syntax(
                            "unclosed block comment",
                            self.make_span(start, line, col),
                        ));
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn next_token(&mut self) -> CompileResult<Token> {
        self.skip_whitespace_and_comments()?;
        let (start, line, col) = self.span_start();

        let Some(ch) = self.peek() else {
            return Ok(Token::new(
                TokenKind::Eof,
                self.make_span(start, line, col),
                "",
            ));
        };

        // Identifiers / keywords
        if ch.is_ascii_alphabetic() || ch == '_' {
            return self.ident_or_keyword(start, line, col);
        }

        // Numbers
        if ch.is_ascii_digit() {
            return self.number(start, line, col);
        }

        // Strings (including $"..." interpolated — stored as StringLit; parser expands {})
        if ch == '$' && self.peek_at(1) == Some('"') {
            self.advance(); // $
            let mut tok = self.string_literal(start, line, col, false)?;
            // Mark as interpolated by prefixing with \x01 sentinel in lexeme metadata via kind
            if let TokenKind::StringLit(s) = tok.kind {
                tok.kind = TokenKind::StringLit(format!("\u{0001}{}", s));
            }
            return Ok(tok);
        }
        if ch == '"' {
            return self.string_literal(start, line, col, false);
        }
        if ch == '@' && self.peek_at(1) == Some('"') {
            self.advance(); // @
            return self.string_literal(start, line, col, true);
        }
        if ch == '\'' {
            return self.char_literal(start, line, col);
        }

        // Multi-char operators
        match (ch, self.peek_at(1), self.peek_at(2)) {
            ('?', Some('?'), Some('=')) => {
                self.advance();
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::QuestionQuestionEq,
                    self.make_span(start, line, col),
                    "??=",
                ));
            }
            ('?', Some('?'), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::QuestionQuestion,
                    self.make_span(start, line, col),
                    "??",
                ));
            }
            ('?', Some('.'), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::QuestionDot,
                    self.make_span(start, line, col),
                    "?.",
                ));
            }
            ('=', Some('>'), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::Arrow,
                    self.make_span(start, line, col),
                    "=>",
                ));
            }
            ('-', Some('>'), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::ThinArrow,
                    self.make_span(start, line, col),
                    "->",
                ));
            }
            ('&', Some('&'), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::AmpAmp,
                    self.make_span(start, line, col),
                    "&&",
                ));
            }
            ('|', Some('|'), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::PipePipe,
                    self.make_span(start, line, col),
                    "||",
                ));
            }
            ('=', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::EqEq,
                    self.make_span(start, line, col),
                    "==",
                ));
            }
            ('!', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::BangEq,
                    self.make_span(start, line, col),
                    "!=",
                ));
            }
            ('<', Some('<'), Some('=')) => {
                self.advance();
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::LtLtEq,
                    self.make_span(start, line, col),
                    "<<=",
                ));
            }
            ('>', Some('>'), Some('=')) => {
                self.advance();
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::GtGtEq,
                    self.make_span(start, line, col),
                    ">>=",
                ));
            }
            ('<', Some('<'), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::LtLt,
                    self.make_span(start, line, col),
                    "<<",
                ));
            }
            ('>', Some('>'), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::GtGt,
                    self.make_span(start, line, col),
                    ">>",
                ));
            }
            ('<', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::LtEq,
                    self.make_span(start, line, col),
                    "<=",
                ));
            }
            ('>', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::GtEq,
                    self.make_span(start, line, col),
                    ">=",
                ));
            }
            ('+', Some('+'), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::PlusPlus,
                    self.make_span(start, line, col),
                    "++",
                ));
            }
            ('-', Some('-'), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::MinusMinus,
                    self.make_span(start, line, col),
                    "--",
                ));
            }
            ('+', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::PlusEq,
                    self.make_span(start, line, col),
                    "+=",
                ));
            }
            ('-', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::MinusEq,
                    self.make_span(start, line, col),
                    "-=",
                ));
            }
            ('*', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::StarEq,
                    self.make_span(start, line, col),
                    "*=",
                ));
            }
            ('/', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::SlashEq,
                    self.make_span(start, line, col),
                    "/=",
                ));
            }
            ('%', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::PercentEq,
                    self.make_span(start, line, col),
                    "%=",
                ));
            }
            ('&', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::AmpEq,
                    self.make_span(start, line, col),
                    "&=",
                ));
            }
            ('|', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::PipeEq,
                    self.make_span(start, line, col),
                    "|=",
                ));
            }
            ('^', Some('='), _) => {
                self.advance();
                self.advance();
                return Ok(Token::new(
                    TokenKind::CaretEq,
                    self.make_span(start, line, col),
                    "^=",
                ));
            }
            _ => {}
        }

        // Single-char
        self.advance();
        let kind = match ch {
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,
            '!' => TokenKind::Bang,
            '=' => TokenKind::Eq,
            '<' => TokenKind::Lt,
            '>' => TokenKind::Gt,
            '&' => TokenKind::Amp,
            '|' => TokenKind::Pipe,
            '^' => TokenKind::Caret,
            '~' => TokenKind::Tilde,
            '?' => TokenKind::Question,
            '.' if self.peek() == Some('.') && self.peek_at(1) == Some('=') => {
                self.advance(); self.advance(); TokenKind::DotDotEq
            }
            '.' if self.peek() == Some('.') => {
                self.advance(); TokenKind::DotDot
            }
            '.' => TokenKind::Dot,
            ',' => TokenKind::Comma,
            ':' => TokenKind::Colon,
            ';' => TokenKind::Semicolon,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            '@' => TokenKind::At,
            _ => {
                return Err(CompileError::syntax(
                    format!("unexpected character '{}'", ch),
                    self.make_span(start, line, col),
                ));
            }
        };

        Ok(Token::new(
            kind,
            self.make_span(start, line, col),
            ch.to_string(),
        ))
    }

    fn ident_or_keyword(
        &mut self,
        start: usize,
        line: usize,
        col: usize,
    ) -> CompileResult<Token> {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        let span = self.make_span(start, line, col);
        let kind = match keyword(&s) {
            Some(TokenKind::True) => TokenKind::BoolLit(true),
            Some(TokenKind::False) => TokenKind::BoolLit(false),
            Some(k) => k,
            None => TokenKind::Ident(s.clone()),
        };
        Ok(Token::new(kind, span, s))
    }

    fn number(&mut self, start: usize, line: usize, col: usize) -> CompileResult<Token> {
        // hex
        if self.peek() == Some('0')
            && matches!(self.peek_at(1), Some('x') | Some('X'))
        {
            self.advance();
            self.advance();
            let mut hex = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() || c == '_' {
                    if c != '_' {
                        hex.push(c);
                    }
                    self.advance();
                } else {
                    break;
                }
            }
            let span = self.make_span(start, line, col);
            let value = u64::from_str_radix(&hex, 16).map_err(|_| {
                CompileError::syntax("invalid hex literal", span)
            })?;
            return Ok(Token::new(TokenKind::UIntLit(value), span, format!("0x{}", hex)));
        }

        let mut int_part = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                if c != '_' {
                    int_part.push(c);
                }
                self.advance();
            } else {
                break;
            }
        }

        let mut is_float = false;
        let mut frac = String::new();
        if self.peek() == Some('.') && self.peek_at(1).map(|c| c.is_ascii_digit()).unwrap_or(false)
        {
            is_float = true;
            self.advance(); // .
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() || c == '_' {
                    if c != '_' {
                        frac.push(c);
                    }
                    self.advance();
                } else {
                    break;
                }
            }
        }

        // exponent
        if matches!(self.peek(), Some('e') | Some('E')) {
            is_float = true;
            let mut exp = String::new();
            exp.push(self.advance().unwrap());
            if matches!(self.peek(), Some('+') | Some('-')) {
                exp.push(self.advance().unwrap());
            }
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    exp.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
            frac.push_str(&exp);
        }

        let span = self.make_span(start, line, col);
        let suffix = self.peek();

        if suffix == Some('f') || suffix == Some('F') {
            self.advance();
            let text = if frac.is_empty() {
                int_part.clone()
            } else {
                format!("{}.{}", int_part, frac)
            };
            let v: f64 = text.parse().map_err(|_| {
                CompileError::syntax("invalid float literal", span)
            })?;
            return Ok(Token::new(TokenKind::FloatLit(v), span, format!("{}f", text)));
        }

        if suffix == Some('m') || suffix == Some('M') {
            self.advance();
            let text = if frac.is_empty() {
                int_part
            } else {
                format!("{}.{}", int_part, frac)
            };
            return Ok(Token::new(TokenKind::DecimalLit(text.clone()), span, format!("{}m", text)));
        }

        if suffix == Some('u') || suffix == Some('U') {
            self.advance();
            let v: u64 = int_part.parse().map_err(|_| {
                CompileError::syntax("invalid uint literal", span)
            })?;
            return Ok(Token::new(TokenKind::UIntLit(v), span, format!("{}u", int_part)));
        }

        if suffix == Some('l') || suffix == Some('L') {
            self.advance();
            let v: i64 = int_part.parse().map_err(|_| {
                CompileError::syntax("invalid long literal", span)
            })?;
            return Ok(Token::new(TokenKind::IntLit(v), span, format!("{}L", int_part)));
        }

        if is_float {
            let cleaned = {
                let mut s = int_part.clone();
                if !frac.is_empty() && !frac.starts_with('e') && !frac.starts_with('E') {
                    // split digits from exponent in frac
                    if let Some(ei) = frac.find(|c: char| c == 'e' || c == 'E') {
                        s.push('.');
                        s.push_str(&frac[..ei]);
                        s.push_str(&frac[ei..]);
                    } else {
                        s.push('.');
                        s.push_str(&frac);
                    }
                } else if !frac.is_empty() {
                    s.push_str(&frac);
                }
                s
            };
            let v: f64 = cleaned.parse().map_err(|_| {
                CompileError::syntax(format!("invalid float literal '{}'", cleaned), span)
            })?;
            return Ok(Token::new(TokenKind::FloatLit(v), span, cleaned));
        }

        let v: i64 = int_part.parse().map_err(|_| {
            CompileError::syntax("invalid integer literal", span)
        })?;
        Ok(Token::new(TokenKind::IntLit(v), span, int_part))
    }

    fn string_literal(
        &mut self,
        start: usize,
        line: usize,
        col: usize,
        raw: bool,
    ) -> CompileResult<Token> {
        self.advance(); // opening "
        let mut s = String::new();
        if raw {
            // verbatim: "" is escaped quote
            loop {
                match self.peek() {
                    None => {
                        return Err(CompileError::syntax(
                            "unclosed string literal",
                            self.make_span(start, line, col),
                        ));
                    }
                    Some('"') if self.peek_at(1) == Some('"') => {
                        self.advance();
                        self.advance();
                        s.push('"');
                    }
                    Some('"') => {
                        self.advance();
                        break;
                    }
                    Some(c) => {
                        s.push(c);
                        self.advance();
                    }
                }
            }
            return Ok(Token::new(
                TokenKind::RawStringLit(s.clone()),
                self.make_span(start, line, col),
                s,
            ));
        }

        loop {
            match self.peek() {
                None => {
                    return Err(CompileError::syntax(
                        "unclosed string literal",
                        self.make_span(start, line, col),
                    ));
                }
                Some('"') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    let esc = self.advance().ok_or_else(|| {
                        CompileError::syntax(
                            "incomplete escape sequence",
                            self.make_span(start, line, col),
                        )
                    })?;
                    s.push(match esc {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '0' => '\0',
                        '\\' => '\\',
                        '"' => '"',
                        '\'' => '\'',
                        '{' => '{',
                        '}' => '}',
                        'u' => {
                            // \uXXXX
                            let mut hex = String::new();
                            for _ in 0..4 {
                                let c = self.advance().ok_or_else(|| {
                                    CompileError::syntax(
                                        "incomplete unicode escape",
                                        self.make_span(start, line, col),
                                    )
                                })?;
                                hex.push(c);
                            }
                            let cp = u32::from_str_radix(&hex, 16).map_err(|_| {
                                CompileError::syntax(
                                    "invalid unicode escape",
                                    self.make_span(start, line, col),
                                )
                            })?;
                            char::from_u32(cp).ok_or_else(|| {
                                CompileError::syntax(
                                    "invalid unicode codepoint",
                                    self.make_span(start, line, col),
                                )
                            })?
                        }
                        other => other,
                    });
                }
                Some(c) => {
                    s.push(c);
                    self.advance();
                }
            }
        }

        // Interpolated strings start with $ — handled at higher level when $ precedes "
        Ok(Token::new(
            TokenKind::StringLit(s.clone()),
            self.make_span(start, line, col),
            s,
        ))
    }

    fn char_literal(
        &mut self,
        start: usize,
        line: usize,
        col: usize,
    ) -> CompileResult<Token> {
        self.advance(); // '
        let ch = match self.advance() {
            Some('\\') => {
                let esc = self.advance().ok_or_else(|| {
                    CompileError::syntax("incomplete char escape", self.make_span(start, line, col))
                })?;
                match esc {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '0' => '\0',
                    '\\' => '\\',
                    '\'' => '\'',
                    '"' => '"',
                    other => other,
                }
            }
            Some(c) => c,
            None => {
                return Err(CompileError::syntax(
                    "unclosed char literal",
                    self.make_span(start, line, col),
                ));
            }
        };
        if self.advance() != Some('\'') {
            return Err(CompileError::syntax(
                "unclosed char literal",
                self.make_span(start, line, col),
            ));
        }
        Ok(Token::new(
            TokenKind::CharLit(ch),
            self.make_span(start, line, col),
            ch.to_string(),
        ))
    }

    pub fn source(&self) -> &str {
        self.source
    }
}

/// Handle `$"..."` interpolated string at lexer level by treating `$` + string specially.
/// The parser can also rewrite Ident("$") patterns; here we expose a helper for the parser.
pub fn is_interpolated_prefix(tokens: &[Token], i: usize) -> bool {
    if i + 1 >= tokens.len() {
        return false;
    }
    matches!(&tokens[i].kind, TokenKind::Ident(s) if s == "$")
        && matches!(tokens[i + 1].kind, TokenKind::StringLit(_))
}
