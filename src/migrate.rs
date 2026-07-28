//! C# → RayTask migration tools (`migrate` / `convert` / `analyze`).

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct AnalyzeReport {
    pub files: Vec<PathBuf>,
    pub issues: Vec<String>,
    pub notes: Vec<String>,
}

/// Convert a single `.cs` source string to RayTask `.rt`.
pub fn convert_csharp_source(cs: &str) -> String {
    let mut out = String::new();
    let mut pending_attrs: Vec<String> = Vec::new();

    for raw in cs.lines() {
        let line = raw;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }

        // Skip assembly / namespace braces handling lightly
        if trimmed.starts_with("namespace ") {
            let name = trimmed
                .trim_start_matches("namespace ")
                .trim_end_matches('{')
                .trim()
                .replace('.', "_");
            out.push_str(&format!("namespace {} {{\n", name));
            continue;
        }

        if trimmed.starts_with("using ") {
            let import = map_using(trimmed);
            if let Some(i) = import {
                out.push_str(&format!("import {};\n", i));
            } else {
                out.push_str(&format!("/* {} */\n", trimmed));
            }
            continue;
        }

        if trimmed.starts_with("[") && trimmed.ends_with("]") {
            pending_attrs.push(map_attribute(trimmed));
            continue;
        }

        for a in pending_attrs.drain(..) {
            out.push_str(&a);
            out.push('\n');
        }

        let mut s = line.to_string();

        // Access modifiers
        s = replace_word(&s, "public ", "export ");
        s = replace_word(&s, "internal ", "");
        s = s.replace("protected internal ", "protected ");

        // Types / keywords
        s = replace_word(&s, "dynamic ", "dyn ");
        s = replace_word(&s, "object ", "dyn ");
        s = s.replace("JsonSerializer.Deserialize", "Json.Parse");
        s = s.replace("JsonSerializer.Serialize", "Json.Stringify");
        s = s.replace("Console.WriteLine", "print");
        s = s.replace("Console.Write", "write");
        s = s.replace("Console.ReadLine()", "readLine()");

        // Method signatures: Type Name(Type a, Type b) → Type Name(a: Type, b: Type)
        if let Some(converted) = convert_signature_line(&s) {
            s = converted;
        }

        out.push_str(&s);
        out.push('\n');
    }

    for a in pending_attrs {
        out.push_str(&a);
        out.push('\n');
    }

    out
}

fn map_using(line: &str) -> Option<&'static str> {
    let u = line
        .trim_start_matches("using ")
        .trim_end_matches(';')
        .trim();
    match u {
        "System" => Some("bstd.io"),
        "System.IO" => Some("bstd.fs"),
        "System.Net" | "System.Net.Http" => Some("bstd.net"),
        "System.Text.Json" | "System.Text" => Some("bstd.json"),
        "System.Collections.Generic" => Some("bstd.collections"),
        "System.Threading.Tasks" => Some("bstd.async"),
        "System.Text.RegularExpressions" => Some("bstd.regex"),
        "System.Security.Cryptography" => Some("bstd.crypto"),
        _ if u.starts_with("System") => Some("bstd.io"),
        _ => None,
    }
}

fn map_attribute(line: &str) -> String {
    let inner = line.trim().trim_start_matches('[').trim_end_matches(']');
    if inner.starts_with("DllImport") {
        // [DllImport("foo")] → [DllImport: "foo"]
        if let Some(start) = inner.find('"') {
            if let Some(end) = inner[start + 1..].find('"') {
                let lib = &inner[start + 1..start + 1 + end];
                return format!("[DllImport: \"{}\"]", lib);
            }
        }
        return "[DllImport: \"\"]".into();
    }
    if inner == "Test" || inner.starts_with("Fact") || inner.starts_with("TestMethod") {
        return "[test]".into();
    }
    format!("/* attr: {} */", inner)
}

fn replace_word(s: &str, from: &str, to: &str) -> String {
    s.replace(from, to)
}

/// Heuristic: convert `Ret Name(T0 a, T1 b)` parameter lists to RayTask style.
fn convert_signature_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    // Skip if already RayTask-ish (`name: type`)
    if trimmed.contains(": ") && trimmed.contains('(') {
        return None;
    }
    let open = trimmed.find('(')?;
    let close = trimmed.rfind(')')?;
    if close <= open {
        return None;
    }
    let params = &trimmed[open + 1..close];
    if params.trim().is_empty() || params.trim() == "void" {
        return None;
    }
    // Don't touch lambdas / foreach
    if trimmed.contains("=>") || trimmed.starts_with("foreach") {
        return None;
    }
    let mut converted = Vec::new();
    for part in params.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        // "int a" / "string? name" / "ref int x" / "out string s"
        let p = p
            .trim_start_matches("ref ")
            .trim_start_matches("out ")
            .trim_start_matches("in ")
            .trim_start_matches("params ");
        let bits: Vec<&str> = p.split_whitespace().collect();
        if bits.len() < 2 {
            converted.push(p.to_string());
            continue;
        }
        let name = bits[bits.len() - 1].trim_end_matches(',').to_string();
        let ty = bits[..bits.len() - 1].join(" ");
        // Skip if looks like call args (no type keywords and lowercase only?)
        if ty.chars().next().map(|c| c.is_lowercase()).unwrap_or(false)
            && !ty.contains('<')
            && !matches!(
                ty.as_str(),
                "string" | "object" | "dynamic" | "var" | "bool" | "byte" | "char"
            )
        {
            return None;
        }
        converted.push(format!("{}: {}", name, ty));
    }
    if converted.is_empty() {
        return None;
    }
    Some(format!(
        "{}{}){}",
        &trimmed[..open + 1],
        converted.join(", "),
        &trimmed[close..]
    ))
}

/// Convert a `.cs` file to `.rt` beside it (or to `output`).
pub fn convert_file(cs_path: &Path, output: Option<&Path>) -> Result<PathBuf, String> {
    let src = fs::read_to_string(cs_path).map_err(|e| e.to_string())?;
    let rt = convert_csharp_source(&src);
    let out = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| cs_path.with_extension("rt"));
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&out, rt).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Parse a `.csproj` for Compile Include paths (simple XML scan).
pub fn csproj_sources(csproj: &Path) -> Result<Vec<PathBuf>, String> {
    let xml = fs::read_to_string(csproj).map_err(|e| e.to_string())?;
    let root = csproj.parent().unwrap_or(Path::new("."));
    let mut files = Vec::new();

    // <Compile Include="Foo.cs" />
    for part in xml.split("Compile").skip(1) {
        if let Some(idx) = part.find("Include=") {
            let rest = &part[idx + "Include=".len()..];
            let quote = rest.chars().next().unwrap_or('"');
            if quote == '"' || quote == '\'' {
                if let Some(end) = rest[1..].find(quote) {
                    let rel = &rest[1..1 + end];
                    let path = root.join(rel.replace('\\', "/"));
                    if path.extension().map(|e| e == "cs").unwrap_or(false) {
                        files.push(path);
                    }
                }
            }
        }
    }

    // SDK-style: include all .cs under project dir
    if files.is_empty() {
        for entry in walkdir_cs(root) {
            files.push(entry);
        }
    }
    Ok(files)
}

fn walkdir_cs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name == "bin" || name == "obj" || name == ".git" {
                    continue;
                }
                stack.push(p);
            } else if p.extension().map(|e| e == "cs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    out
}

/// Migrate a C# project into a RayTask project directory.
pub fn migrate_csproj(csproj: &Path, out_dir: Option<&Path>) -> Result<PathBuf, String> {
    let sources = csproj_sources(csproj)?;
    if sources.is_empty() {
        return Err("no .cs sources found".into());
    }
    let name = csproj
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("migrated");
    let dest = out_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| csproj.parent().unwrap_or(Path::new(".")).join(format!("{}_rt", name)));
    fs::create_dir_all(dest.join("src")).map_err(|e| e.to_string())?;

    let mut converted = Vec::new();
    for cs in &sources {
        let rel = cs
            .strip_prefix(csproj.parent().unwrap_or(Path::new(".")))
            .unwrap_or(cs.as_path());
        let target = dest.join("src").join(rel).with_extension("rt");
        convert_file(cs, Some(&target))?;
        converted.push(target);
    }

    // Prefer Program.cs / Main as entry
    let entry = converted
        .iter()
        .find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("Program") || s.eq_ignore_ascii_case("Main"))
                .unwrap_or(false)
        })
        .cloned()
        .unwrap_or_else(|| converted[0].clone());
    let entry_rel = entry
        .strip_prefix(&dest)
        .unwrap_or(&entry)
        .to_string_lossy()
        .replace('\\', "/");

    fs::write(
        dest.join("project.rtp"),
        format!(
            r#"project "{name}" {{
    version = "0.1.0"
    author = ""
    description = "Migrated from {csproj}"
    entry = "{entry_rel}"

    dependencies {{
    }}

    build {{
        optimize = "speed"
        target = "bytecode"
        gc = true
    }}
}}
"#,
            csproj = csproj.display()
        ),
    )
    .map_err(|e| e.to_string())?;

    fs::write(
        dest.join("MIGRATION.md"),
        format!(
            "# Migration from {}\n\nConverted {} file(s).\n\nReview signatures, `using` → `import`, and `public` → `export`.\n",
            csproj.display(),
            converted.len()
        ),
    )
    .map_err(|e| e.to_string())?;

    Ok(dest)
}

pub fn analyze_csproj(csproj: &Path) -> Result<AnalyzeReport, String> {
    let sources = csproj_sources(csproj)?;
    let mut report = AnalyzeReport {
        files: sources.clone(),
        ..Default::default()
    };
    report
        .notes
        .push(format!("Found {} C# source file(s)", sources.len()));

    let mut unsupported = 0usize;
    for cs in &sources {
        let Ok(src) = fs::read_to_string(cs) else {
            report.issues.push(format!("cannot read {}", cs.display()));
            continue;
        };
        for (i, line) in src.lines().enumerate() {
            let t = line.trim();
            for needle in [
                "async void",
                "IEnumerable",
                "yield return",
                "LINQ",
                "unsafe fixed",
                "Span<",
                "ref struct",
                "record ",
                "required ",
                "nameof(",
                "nameof ",
                "global using",
                "file scoped",
            ] {
                if t.contains(needle) {
                    unsupported += 1;
                    report.issues.push(format!(
                        "{}:{}: possible manual review — `{}`",
                        cs.display(),
                        i + 1,
                        needle.trim()
                    ));
                    break;
                }
            }
        }
    }
    report.notes.push(format!(
        "Compatibility scan flagged {} line(s) for review",
        unsupported
    ));
    report.notes.push(
        "Automatic convert maps: public→export, using System*→import bstd.*, param lists, dynamic→dyn".into(),
    );
    Ok(report)
}
