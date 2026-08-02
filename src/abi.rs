//! C ABI layout for `[repr: "C"]` structs (and packed / aligned variants).

use crate::ast::{Member, StructDecl, TypeRef, UnionDecl};
use crate::ffi::{FfiFieldLayout, FfiStructLayout, FfiType};

/// Primitive size/align in bytes (ILP32/LP64-ish; pointers = 8 on host VM).
pub fn primitive_size_align(name: &str) -> Option<(usize, usize)> {
    Some(match name {
        "void" => (0, 1),
        "bool" => (1, 1),
        "byte" | "ubyte" | "i8" | "u8" | "sbyte" => (1, 1),
        "short" | "ushort" | "i16" | "u16" => (2, 2),
        "char" => (2, 2),
        "int" | "uint" | "i32" | "u32" | "float" | "f32" => (4, 4),
        "long" | "ulong" | "i64" | "u64" | "double" | "f64" => (8, 8),
        "ptr" | "pointer" | "nuint" | "nint" | "string" | "str" => (8, 8),
        _ => return None,
    })
}

fn align_up(off: usize, align: usize) -> usize {
    if align == 0 {
        return off;
    }
    (off + align - 1) & !(align - 1)
}

fn field_type_size_align(
    ty: &TypeRef,
    known: &std::collections::HashMap<String, FfiStructLayout>,
) -> Option<(usize, usize, FfiType)> {
    if let Some(layout) = known.get(&ty.name) {
        return Some((
            layout.size,
            layout.align,
            FfiType::Struct(layout.clone()),
        ));
    }
    let ffi = FfiType::from_type_ref(ty);
    if matches!(ffi, FfiType::Ptr) && primitive_size_align(&ty.name).is_none() {
        // Unknown named type without layout → treat as pointer
        return Some((8, 8, FfiType::Ptr));
    }
    let (sz, al) = match &ffi {
        FfiType::Void => (0, 1),
        FfiType::Bool | FfiType::I8 | FfiType::U8 => (1, 1),
        FfiType::I16 | FfiType::U16 => (2, 2),
        FfiType::I32 | FfiType::U32 | FfiType::F32 => (4, 4),
        FfiType::I64 | FfiType::U64 | FfiType::F64 | FfiType::Ptr | FfiType::CString => (8, 8),
        FfiType::Struct(s) | FfiType::StructPtr(s) => (s.size, s.align),
    };
    Some((sz, al, ffi))
}

/// Compute C layout for a RayTask struct declaration.
pub fn layout_struct(
    s: &StructDecl,
    known: &std::collections::HashMap<String, FfiStructLayout>,
) -> FfiStructLayout {
    let mut fields = Vec::new();
    let mut offset = 0usize;
    let mut max_align = 1usize;

    if s.packed {
        max_align = 1;
    }

    for m in &s.members {
        let Member::Field(f) = m else {
            continue;
        };
        if f.is_static {
            continue;
        }
        let Some(ty) = &f.ty else {
            continue;
        };
        let Some((sz, al, ffi_ty)) = field_type_size_align(ty, known) else {
            continue;
        };
        let al = if s.packed { 1 } else { al };
        if !s.packed {
            max_align = max_align.max(al);
            offset = align_up(offset, al);
        }
        fields.push(FfiFieldLayout {
            name: f.name.clone(),
            offset,
            ty: ffi_ty,
        });
        offset += sz;
    }

    if let Some(a) = s.align {
        max_align = max_align.max(a as usize);
    }
    let size = if s.packed {
        offset
    } else {
        align_up(offset, max_align.max(1))
    };

    FfiStructLayout {
        name: s.name.clone(),
        size,
        align: max_align.max(1),
        fields,
        packed: s.packed,
    }
}

/// Union layout: size = max member, offset 0 for all, align = max align.
pub fn layout_union(
    u: &UnionDecl,
    known: &std::collections::HashMap<String, FfiStructLayout>,
) -> FfiStructLayout {
    let mut fields = Vec::new();
    let mut size = 0usize;
    let mut max_align = 1usize;
    for m in &u.members {
        let Member::Field(f) = m else {
            continue;
        };
        let Some(ty) = &f.ty else {
            continue;
        };
        let Some((sz, al, ffi_ty)) = field_type_size_align(ty, known) else {
            continue;
        };
        let al = if u.packed { 1 } else { al };
        max_align = max_align.max(al);
        size = size.max(sz);
        fields.push(FfiFieldLayout {
            name: f.name.clone(),
            offset: 0,
            ty: ffi_ty,
        });
    }
    if let Some(a) = u.align {
        max_align = max_align.max(a as usize);
    }
    if !u.packed {
        size = align_up(size, max_align.max(1));
    }
    FfiStructLayout {
        name: u.name.clone(),
        size,
        align: max_align.max(1),
        fields,
        packed: u.packed,
    }
}

/// Win64 / common rule: aggregates of size 1/2/4/8 pass in an integer register;
/// larger aggregates pass/return via hidden pointer (by-value ABI).
pub fn struct_fits_register(layout: &FfiStructLayout) -> bool {
    matches!(layout.size, 1 | 2 | 4 | 8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Access, FieldDecl};
    use crate::span::Span;

    fn field(name: &str, ty: &str) -> Member {
        Member::Field(FieldDecl {
            access: Access::Default,
            is_static: false,
            is_const: false,
            ty: Some(TypeRef::named(ty, Span::default())),
            name: name.into(),
            init: None,
            span: Span::default(),
        })
    }

    #[test]
    fn point_layout() {
        let s = StructDecl {
            access: Access::Default,
            name: "Point".into(),
            type_params: vec![],
            members: vec![field("x", "int"), field("y", "int")],
            attributes: vec![],
            packed: false,
            align: None,
            repr_c: true,
            span: Span::default(),
        };
        let layout = layout_struct(&s, &Default::default());
        assert_eq!(layout.size, 8);
        assert_eq!(layout.align, 4);
        assert_eq!(layout.fields[0].offset, 0);
        assert_eq!(layout.fields[1].offset, 4);
    }

    #[test]
    fn packed_layout() {
        let s = StructDecl {
            access: Access::Default,
            name: "P".into(),
            type_params: vec![],
            members: vec![field("a", "byte"), field("b", "int")],
            attributes: vec![],
            packed: true,
            align: None,
            repr_c: true,
            span: Span::default(),
        };
        let layout = layout_struct(&s, &Default::default());
        assert_eq!(layout.size, 5);
        assert_eq!(layout.fields[1].offset, 1);
    }
}
