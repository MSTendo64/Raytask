//! RayTask type system.

use crate::ast::TypeRef;
use crate::span::Span;
use std::fmt;

/// Canonical type used by the typechecker.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    Void,
    Bool,
    Byte,
    SByte,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
    Float,
    Double,
    Decimal,
    Char,
    String,
    /// Dynamic type — escapes static checking.
    Dyn,
    /// Bottom type of `null` literal.
    Null,
    /// User-defined or unresolved named type.
    Named(String),
    /// Generic instantiation, e.g. `List<int>`, `Dictionary<string, int>`.
    Generic { name: String, args: Vec<Ty> },
    /// Array / multi-dimensional array.
    Array { elem: Box<Ty>, dims: usize },
    /// Nullable wrapper `T?`.
    Nullable(Box<Ty>),
    /// Pointer `ptr<T>`.
    Ptr(Box<Ty>),
    /// Function / lambda type.
    Func { params: Vec<Ty>, ret: Box<Ty> },
    /// Generic type parameter in scope.
    TypeParam(String),
    /// Error recovery type.
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCategory {
    Value,
    Reference,
    Nullable,
    Pointer,
    Dynamic,
    TypeParameter,
    Error,
}

impl Ty {
    pub fn named(name: impl Into<String>) -> Self {
        Ty::Named(name.into())
    }

    pub fn list(elem: Ty) -> Self {
        Ty::Generic {
            name: "List".into(),
            args: vec![elem],
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Ty::Byte
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
                | Ty::Dyn
                | Ty::Error
        )
    }

    pub fn is_integral(&self) -> bool {
        matches!(
            self,
            Ty::Byte
                | Ty::SByte
                | Ty::Short
                | Ty::UShort
                | Ty::Int
                | Ty::UInt
                | Ty::Long
                | Ty::ULong
                | Ty::Dyn
                | Ty::Error
        )
    }

    pub fn is_floating(&self) -> bool {
        matches!(
            self,
            Ty::Float | Ty::Double | Ty::Decimal | Ty::Dyn | Ty::Error
        )
    }

    pub fn is_bool_like(&self) -> bool {
        matches!(self, Ty::Bool | Ty::Dyn | Ty::Error)
    }

    pub fn category(&self) -> TypeCategory {
        match self {
            Ty::Void
            | Ty::Bool
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
            | Ty::Char => TypeCategory::Value,
            Ty::String | Ty::Named(_) | Ty::Generic { .. } | Ty::Array { .. } | Ty::Func { .. } => {
                TypeCategory::Reference
            }
            Ty::Nullable(_) | Ty::Null => TypeCategory::Nullable,
            Ty::Ptr(_) => TypeCategory::Pointer,
            Ty::Dyn => TypeCategory::Dynamic,
            Ty::TypeParam(_) => TypeCategory::TypeParameter,
            Ty::Error => TypeCategory::Error,
        }
    }

    pub fn is_nullable(&self) -> bool {
        matches!(
            self.category(),
            TypeCategory::Reference
                | TypeCategory::Nullable
                | TypeCategory::Pointer
                | TypeCategory::Dynamic
                | TypeCategory::Error
        )
    }

    pub fn unwrap_nullable(&self) -> &Ty {
        match self {
            Ty::Nullable(inner) => inner,
            other => other,
        }
    }

    pub fn make_nullable(self) -> Ty {
        match self {
            Ty::Nullable(_) | Ty::Dyn | Ty::Null | Ty::Error => self,
            other => Ty::Nullable(Box::new(other)),
        }
    }

    /// Numeric rank for promotions (higher wins).
    pub fn numeric_rank(&self) -> Option<u8> {
        Some(match self {
            Ty::Byte | Ty::SByte => 1,
            Ty::Short | Ty::UShort => 2,
            Ty::Int | Ty::UInt => 3,
            Ty::Long | Ty::ULong => 4,
            Ty::Float => 5,
            Ty::Double => 6,
            Ty::Decimal => 7,
            Ty::Dyn | Ty::Error => 100,
            _ => return None,
        })
    }

    pub fn promote_numeric(a: &Ty, b: &Ty) -> Option<Ty> {
        if matches!(a, Ty::Dyn) || matches!(b, Ty::Dyn) {
            return Some(Ty::Dyn);
        }
        if matches!(a, Ty::Error) || matches!(b, Ty::Error) {
            return Some(Ty::Error);
        }
        let ra = a.numeric_rank()?;
        let rb = b.numeric_rank()?;
        if ra >= rb {
            Some(a.clone())
        } else {
            Some(b.clone())
        }
    }

    /// Can `from` be assigned to a variable of type `to`?
    pub fn is_assignable_to(&self, to: &Ty) -> bool {
        if self == to {
            return true;
        }
        if matches!(to, Ty::Dyn | Ty::Error) || matches!(self, Ty::Dyn | Ty::Error) {
            return true;
        }
        // null → any nullable / reference
        if matches!(self, Ty::Null) {
            return to.is_nullable() || matches!(to, Ty::Nullable(_));
        }
        // T → T?
        if let Ty::Nullable(inner) = to {
            if self.is_assignable_to(inner) || matches!(self, Ty::Null) {
                return true;
            }
        }
        // T? → T (not allowed without check) — only Dyn bypasses
        if let Ty::Nullable(inner) = self {
            // Allow T? → T? already handled; T? → U if T→U only via Dyn
            if let Ty::Nullable(to_inner) = to {
                return inner.is_assignable_to(to_inner);
            }
            return false;
        }
        // numeric widening
        if let (Some(rf), Some(rt)) = (self.numeric_rank(), to.numeric_rank()) {
            // allow same-ish widening (int→long, float→double, etc.)
            if rf <= rt {
                // disallow float→decimal silently? allow all upward
                return true;
            }
            // allow explicit narrowing only via cast — not here
        }
        // array covariance (simplified: exact elem match or dyn)
        match (self, to) {
            (
                Ty::Array {
                    elem: a,
                    dims: da,
                },
                Ty::Array {
                    elem: b,
                    dims: db,
                },
            ) if da == db => return a.is_assignable_to(b),
            (
                Ty::Generic {
                    name: na,
                    args: aa,
                },
                Ty::Generic {
                    name: nb,
                    args: ab,
                },
            ) if na == nb && aa.len() == ab.len() => {
                return aa.iter().zip(ab.iter()).all(|(x, y)| x.is_assignable_to(y));
            }
            (Ty::Ptr(a), Ty::Ptr(b)) => return a.is_assignable_to(b),
            (
                Ty::Func {
                    params: pa,
                    ret: ra,
                },
                Ty::Func {
                    params: pb,
                    ret: rb,
                },
            ) if pa.len() == pb.len() => {
                // contravariant params, covariant return (simplified: invariant params)
                return pa.iter().zip(pb.iter()).all(|(a, b)| b.is_assignable_to(a))
                    && ra.is_assignable_to(rb);
            }
            _ => {}
        }
        // Named identity already checked; inheritance handled by typechecker env
        false
    }

    /// Unify two branch types (ternary / if expression).
    pub fn unify(a: &Ty, b: &Ty) -> Ty {
        if a == b {
            return a.clone();
        }
        if matches!(a, Ty::Error) {
            return b.clone();
        }
        if matches!(b, Ty::Error) {
            return a.clone();
        }
        if matches!(a, Ty::Dyn) || matches!(b, Ty::Dyn) {
            return Ty::Dyn;
        }
        if matches!(a, Ty::Null) {
            return b.clone().make_nullable();
        }
        if matches!(b, Ty::Null) {
            return a.clone().make_nullable();
        }
        if let Some(p) = Ty::promote_numeric(a, b) {
            return p;
        }
        if a.is_assignable_to(b) {
            return b.clone();
        }
        if b.is_assignable_to(a) {
            return a.clone();
        }
        Ty::Dyn
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Void => write!(f, "void"),
            Ty::Bool => write!(f, "bool"),
            Ty::Byte => write!(f, "byte"),
            Ty::SByte => write!(f, "sbyte"),
            Ty::Short => write!(f, "short"),
            Ty::UShort => write!(f, "ushort"),
            Ty::Int => write!(f, "int"),
            Ty::UInt => write!(f, "uint"),
            Ty::Long => write!(f, "long"),
            Ty::ULong => write!(f, "ulong"),
            Ty::Float => write!(f, "float"),
            Ty::Double => write!(f, "double"),
            Ty::Decimal => write!(f, "decimal"),
            Ty::Char => write!(f, "char"),
            Ty::String => write!(f, "string"),
            Ty::Dyn => write!(f, "dyn"),
            Ty::Null => write!(f, "null"),
            Ty::Named(n) => write!(f, "{}", n),
            Ty::Generic { name, args } => {
                write!(f, "{}<", name)?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", a)?;
                }
                write!(f, ">")
            }
            Ty::Array { elem, dims } => {
                write!(f, "{}", elem)?;
                write!(f, "[")?;
                for i in 0..*dims {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                }
                write!(f, "]")
            }
            Ty::Nullable(inner) => write!(f, "{}?", inner),
            Ty::Ptr(inner) => write!(f, "ptr<{}>", inner),
            Ty::Func { params, ret } => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") => {}", ret)
            }
            Ty::TypeParam(n) => write!(f, "{}", n),
            Ty::Error => write!(f, "<error>"),
        }
    }
}

/// Convert AST type reference into canonical `Ty`.
pub fn ty_from_ref(tr: &TypeRef) -> Ty {
    let mut base = match tr.name.as_str() {
        "void" => Ty::Void,
        "bool" => Ty::Bool,
        "byte" => Ty::Byte,
        "sbyte" => Ty::SByte,
        "short" => Ty::Short,
        "ushort" => Ty::UShort,
        "int" => Ty::Int,
        "uint" => Ty::UInt,
        "long" => Ty::Long,
        "ulong" => Ty::ULong,
        "float" => Ty::Float,
        "double" => Ty::Double,
        "decimal" => Ty::Decimal,
        "char" => Ty::Char,
        "string" => Ty::String,
        "dyn" | "var" => Ty::Dyn,
        "ptr" => {
            let inner = tr
                .args
                .first()
                .map(ty_from_ref)
                .unwrap_or(Ty::Void);
            Ty::Ptr(Box::new(inner))
        }
        other => {
            if tr.args.is_empty() {
                Ty::Named(other.to_string())
            } else {
                Ty::Generic {
                    name: other.to_string(),
                    args: tr.args.iter().map(ty_from_ref).collect(),
                }
            }
        }
    };

    if tr.is_array {
        base = Ty::Array {
            elem: Box::new(base),
            dims: tr.array_dims.max(1),
        };
    }
    if tr.nullable {
        base = base.make_nullable();
    }
    base
}

pub fn ty_from_ref_span(tr: &TypeRef) -> (Ty, Span) {
    (ty_from_ref(tr), tr.span)
}

/// Built-in global function signatures: (name, params, return).
pub fn builtin_functions() -> Vec<(&'static str, Vec<Ty>, Ty)> {
    vec![
        ("print", vec![Ty::Dyn], Ty::Void),
        ("write", vec![Ty::Dyn], Ty::Void),
        ("readLine", vec![], Ty::String),
        ("readKey", vec![], Ty::Char),
        ("sleep", vec![Ty::Int], Ty::Void),
        ("ParseInt", vec![Ty::String], Ty::Int),
        ("ParseFloat", vec![Ty::String], Ty::Float),
        ("ToString", vec![Ty::Dyn], Ty::String),
        ("IsNull", vec![Ty::Dyn], Ty::Bool),
        ("IsNotNull", vec![Ty::Dyn], Ty::Bool),
        ("IsNumeric", vec![Ty::String], Ty::Bool),
        ("IsAlpha", vec![Ty::String], Ty::Bool),
        ("IsEmail", vec![Ty::String], Ty::Bool),
        ("RandomInt", vec![Ty::Int, Ty::Int], Ty::Int),
        ("GenerateGuid", vec![], Ty::String),
        ("GetTime", vec![], Ty::Long),
        ("assert", vec![Ty::Bool], Ty::Void),
        ("assertEq", vec![Ty::Dyn, Ty::Dyn], Ty::Void),
        ("gc", vec![], Ty::Int),
        ("malloc", vec![Ty::Int], Ty::Ptr(Box::new(Ty::Byte))),
        ("free", vec![Ty::Ptr(Box::new(Ty::Byte))], Ty::Void),
        ("sizeof", vec![Ty::Dyn], Ty::Int),
        ("MmioRead32", vec![Ty::Long], Ty::Int),
        ("MmioWrite32", vec![Ty::Long, Ty::Int], Ty::Void),
        ("Spin", vec![Ty::Int], Ty::Void),
        ("IsFreestanding", vec![], Ty::Bool),
        ("regex", vec![Ty::String], Ty::Named("Regex".into())),
        ("Ok", vec![Ty::Dyn], Ty::Generic {
            name: "Result".into(),
            args: vec![Ty::Dyn, Ty::Dyn],
        }),
        ("Error", vec![Ty::Dyn], Ty::Generic {
            name: "Result".into(),
            args: vec![Ty::Dyn, Ty::Dyn],
        }),
    ]
}
