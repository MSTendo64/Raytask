//! Multi-file import resolution for RayTask programs.

use crate::ast::{ImportDecl, Item, Program};
use crate::error::{CompileError, CompileResult};
use crate::lexer::Lexer;
use crate::parser::Parser;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Resolve `import` declarations by loading `.rt` files and merging their items
/// ahead of the main program (stdlib and relative modules first).
pub fn resolve_program(source: &str, entry_path: Option<&Path>) -> CompileResult<Program> {
    resolve_program_with_stdlib(source, entry_path, true)
}

pub fn resolve_program_with_stdlib(
    source: &str,
    entry_path: Option<&Path>,
    stdlib_enabled: bool,
) -> CompileResult<Program> {
    let entry_dir = entry_path
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let mut loaded = HashSet::new();
    let mut merged = Vec::new();
    let program = parse_str(source)?;
    resolve_imports(&program, &entry_dir, &mut loaded, &mut merged, stdlib_enabled)?;
    // Keep non-import items from dependencies, then main (imports become no-ops markers)
    let mut out_items = merged;
    out_items.extend(program.items);
    Ok(Program { items: out_items })
}

fn parse_str(source: &str) -> CompileResult<Program> {
    let tokens = Lexer::new(source).tokenize()?;
    Parser::new(tokens).parse()
}

fn resolve_imports(
    program: &Program,
    base_dir: &Path,
    loaded: &mut HashSet<String>,
    out: &mut Vec<Item>,
    stdlib_enabled: bool,
) -> CompileResult<()> {
    for item in &program.items {
        collect_from_item(item, base_dir, loaded, out, stdlib_enabled)?;
    }
    Ok(())
}

fn collect_from_item(
    item: &Item,
    base_dir: &Path,
    loaded: &mut HashSet<String>,
    out: &mut Vec<Item>,
    stdlib_enabled: bool,
) -> CompileResult<()> {
    match item {
        Item::Import(imp) => load_import(imp, base_dir, loaded, out, stdlib_enabled),
        Item::Attribute(_, inner) => collect_from_item(inner, base_dir, loaded, out, stdlib_enabled),
        Item::Namespace(ns) => {
            for i in &ns.items {
                collect_from_item(i, base_dir, loaded, out, stdlib_enabled)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn load_import(
    imp: &ImportDecl,
    base_dir: &Path,
    loaded: &mut HashSet<String>,
    out: &mut Vec<Item>,
    stdlib_enabled: bool,
) -> CompileResult<()> {
    let key = imp.path.clone();
    if loaded.contains(&key) {
        return Ok(());
    }
    loaded.insert(key.clone());

    let path = resolve_path(&imp.path, base_dir, stdlib_enabled)?;
    let Some(path) = path else {
        // Unknown import (e.g. pure marker for builtins like bstd.io) — OK
        return Ok(());
    };

    // If the resolved path is a directory (@-import), load all .rt files
    if path.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(&path)
            .map_err(|e| CompileError::Io {
                message: format!("cannot read directory '{}': {}", path.display(), e),
            })?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "rt"))
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in &entries {
            let file_path = entry.path();
            let file_key = file_path.display().to_string();
            if loaded.contains(&file_key) {
                continue;
            }
            loaded.insert(file_key.clone());
            let source = std::fs::read_to_string(&file_path).map_err(|e| CompileError::Io {
                message: format!("cannot read import '{}': {}", file_path.display(), e),
            })?;
            let dep_dir = file_path.parent().unwrap_or(base_dir);
            let dep = parse_str(&source)?;
            resolve_imports(&dep, dep_dir, loaded, out, stdlib_enabled)?;
            for item in dep.items {
                match &item {
                    Item::Import(_) => {}
                    _ => out.push(item),
                }
            }
        }
        return Ok(());
    }

    let source = std::fs::read_to_string(&path).map_err(|e| CompileError::Io {
        message: format!("cannot read import '{}': {}", path.display(), e),
    })?;
    let dep_dir = path.parent().unwrap_or(base_dir);
    let dep = parse_str(&source)?;
    // Recurse first so transitive deps come earlier
    resolve_imports(&dep, dep_dir, loaded, out, stdlib_enabled)?;
    for item in dep.items {
        match &item {
            Item::Import(_) => {}
            _ => out.push(item),
        }
    }
    Ok(())
}

fn resolve_path(
    import_path: &str,
    base_dir: &Path,
    stdlib_enabled: bool,
) -> CompileResult<Option<PathBuf>> {
    // @-prefixed absolute path: import @C:/path/to/src or import @"C:/path"
    if import_path.starts_with('@') {
        let raw = &import_path[1..];
        let p = PathBuf::from(raw);
        // Try as a directory (import directory → all .rt files)
        if p.is_dir() {
            return Ok(Some(p));
        }
        // Try as a file directly
        if p.is_file() {
            return Ok(Some(p));
        }
        // Try with .rt extension
        let with_ext = PathBuf::from(format!("{}.rt", raw));
        if with_ext.is_file() {
            return Ok(Some(with_ext));
        }
        return Err(CompileError::resolve(
            format!("cannot resolve import '{}'", import_path),
            crate::span::Span::default(),
        ));
    }

    // bstd.* is provided by VM natives; `.rt` stubs are API docs only.
    if import_path.starts_with("bstd.") || import_path == "bstd" {
        if stdlib_enabled {
            return Ok(None);
        }
        return Err(CompileError::resolve(
            format!("cannot resolve import '{}' with --no-stdlib", import_path),
            crate::span::Span::default(),
        ));
    }

    // Relative / package path: foo.bar → foo/bar.rt or foo.bar.rt
    let rel = import_path.replace('.', "/");
    let pkg_root = PathBuf::from(".raytask").join("packages");
    let first = import_path.split('.').next().unwrap_or(import_path);
    let candidates = [
        base_dir.join(format!("{}.rt", rel)),
        base_dir.join(format!("{}.rt", import_path)),
        base_dir.join(&rel).join("mod.rt"),
        PathBuf::from(format!("{}.rt", rel)),
        // Local package manager installs
        pkg_root.join(first).join("src").join("lib.rt"),
        pkg_root.join(first).join("src").join(format!("{}.rt", rel.trim_start_matches(first).trim_start_matches('.').replace('.', "/"))),
        pkg_root.join(first).join(format!("{}.rt", rel)),
        pkg_root.join(import_path).join("src").join("lib.rt"),
        pkg_root.join(first).join("lib.rt"),
    ];
    for c in &candidates {
        if c.is_file() {
            return Ok(Some(c.clone()));
        }
    }
    // Also try package src/<rest>.rt
    let rest: Vec<_> = import_path.split('.').skip(1).collect();
    if !rest.is_empty() {
        let p = pkg_root
            .join(first)
            .join("src")
            .join(format!("{}.rt", rest.join("/")));
        if p.is_file() {
            return Ok(Some(p));
        }
    }
    Err(CompileError::resolve(
        format!("cannot resolve import '{}'", import_path),
        crate::span::Span::default(),
    ))
}
