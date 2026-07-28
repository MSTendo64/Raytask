//! Compiler and runtime errors.

use crate::span::Span;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum CompileError {
    #[error("{span}: {message}")]
    Syntax { message: String, span: Span },

    #[error("{span}: {message}")]
    Type { message: String, span: Span },

    #[error("{span}: {message}")]
    Resolve { message: String, span: Span },

    #[error("{message}")]
    Io { message: String },

    #[error("{message}")]
    Internal { message: String },
}

impl CompileError {
    pub fn syntax(message: impl Into<String>, span: Span) -> Self {
        Self::Syntax {
            message: message.into(),
            span,
        }
    }

    pub fn type_err(message: impl Into<String>, span: Span) -> Self {
        Self::Type {
            message: message.into(),
            span,
        }
    }

    pub fn resolve(message: impl Into<String>, span: Span) -> Self {
        Self::Resolve {
            message: message.into(),
            span,
        }
    }
}

pub type CompileResult<T> = Result<T, CompileError>;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("runtime error: {0}")]
    Message(String),

    #[error("stack underflow")]
    StackUnderflow,

    #[error("undefined variable: {0}")]
    UndefinedVariable(String),

    #[error("type error: {0}")]
    TypeError(String),

    #[error("index out of range")]
    IndexOutOfRange,

    #[error("division by zero")]
    DivisionByZero,

    #[error("null reference")]
    NullReference,

    #[error("uncaught exception: {0}")]
    Exception(String),
}

pub type RuntimeResult<T> = Result<T, RuntimeError>;
