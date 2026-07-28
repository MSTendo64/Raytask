//! Product targets, migrate, remote registry, native async/GC.

use raytask::codegen_c::{CodegenOptions, RuntimeProfile};
use raytask::migrate::{analyze_csproj, convert_csharp_source, convert_file};
use raytask::project::{publish_package, search_packages};
use raytask::{compile_file, transpile_c_with, BuildOptions, Target};
use std::fs;
use std::path::PathBuf;

fn tmp_dir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("raytask_product_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn convert_csharp_basics() {
    let cs = r#"
using System;
using System.Collections.Generic;

public class Greeter {
    public string Hello(string name) {
        Console.WriteLine(name);
        return name;
    }
}
"#;
    let rt = convert_csharp_source(cs);
    assert!(rt.contains("import bstd.io"));
    assert!(rt.contains("export class Greeter"));
    assert!(rt.contains("name: string") || rt.contains("Hello(name: string)"));
}

#[test]
fn migrate_and_analyze_csproj() {
    let dir = tmp_dir("migrate");
    let cs = dir.join("Hello.cs");
    fs::write(
        &cs,
        "using System;\npublic class Program {\n  public static void Main() { Console.WriteLine(\"hi\"); }\n}\n",
    )
    .unwrap();
    let csproj = dir.join("Hello.csproj");
    fs::write(
        &csproj,
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net8.0</TargetFramework></PropertyGroup>
</Project>
"#,
    )
    .unwrap();

    let report = analyze_csproj(&csproj).unwrap();
    assert!(!report.files.is_empty());

    let out = dir.join("out_rt");
    let dest = raytask::migrate::migrate_csproj(&csproj, Some(&out)).unwrap();
    assert!(dest.join("project.rtp").exists());
    assert!(dest.join("src").exists());
}

#[test]
fn publish_and_search_local_registry() {
    let dir = tmp_dir("reg");
    let pkg = dir.join("CoolLib");
    fs::create_dir_all(pkg.join("src")).unwrap();
    fs::write(
        pkg.join("package.rtp"),
        "package \"CoolLib\" {\n    version = \"1.2.3\"\n}\n",
    )
    .unwrap();
    fs::write(pkg.join("src/lib.rt"), "export string Name() => \"CoolLib\";\n").unwrap();

    let cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&dir).unwrap();
    let msg = publish_package(&pkg).unwrap();
    assert!(msg.contains("CoolLib"));
    let found = search_packages("Cool").unwrap();
    assert!(found.iter().any(|p| p.name.contains("CoolLib")));
    std::env::set_current_dir(cwd).unwrap();
}

#[test]
fn native_runtime_has_async_and_gc() {
    let src = r#"
import bstd.io;

async int Work() {
    await Task.Delay(1);
    return 42;
}

void Main() {
    var t = Work();
    print("ok");
    Gc.Collect();
}
"#;
    let c = transpile_c_with(
        src,
        CodegenOptions {
            profile: RuntimeProfile::Host,
            gc: true,
            freestanding: false,
        },
    )
    .unwrap();
    assert!(c.contains("rt_await") || c.contains("await("));
    assert!(c.contains("Gc_Collect"));
    assert!(c.contains("rt_gc_alloc") || c.contains("rt_zalloc"));
    assert!(c.contains("RtTask"));
}

#[test]
fn kernel_target_emits_freestanding() {
    let dir = tmp_dir("kernel");
    let src = dir.join("k.rt");
    fs::write(
        &src,
        r#"
[export: "kmain"]
void KernelMain() {
}

void Main() {
    KernelMain();
}
"#,
    )
    .unwrap();
    let out = compile_file(
        src.to_str().unwrap(),
        &BuildOptions {
            target: Target::Kernel,
            gc: false,
            ..BuildOptions::default()
        },
    )
    .unwrap();
    let c = fs::read_to_string(&out).unwrap();
    assert!(c.contains("freestanding") || c.contains("kmain") || c.contains("KernelMain"));
    assert!(c.contains("Gc_Collect")); // stub present
}

#[test]
fn wasm_and_web_and_mobile_scaffolds() {
    let dir = tmp_dir("targets");
    let src = dir.join("app.rt");
    fs::write(
        &src,
        "import bstd.io;\nvoid Main() { print(\"hi\"); }\n",
    )
    .unwrap();

    for target in [Target::Wasm, Target::Web, Target::Mobile, Target::Embedded] {
        let out = compile_file(
            src.to_str().unwrap(),
            &BuildOptions {
                target,
                ..BuildOptions::default()
            },
        )
        .unwrap();
        assert!(!out.is_empty(), "{:?} empty", target);
        let p = PathBuf::from(&out);
        assert!(p.exists(), "{:?} missing {}", target, out);
    }
}

#[test]
fn interrupt_and_address_attrs_in_c() {
    let src = r#"
[address: 0x40021000]
struct Gpio {
    int mode;
}

[interrupt: 0x80]
void Handler() {
}

void Main() {
}
"#;
    let c = transpile_c_with(
        src,
        CodegenOptions {
            profile: RuntimeProfile::Kernel,
            gc: false,
            freestanding: true,
        },
    )
    .unwrap();
    assert!(c.contains("0x40021000") || c.contains("Gpio_ADDR"));
    assert!(c.contains("0x80") || c.contains("isr"));
}

#[test]
fn convert_file_writes_rt() {
    let dir = tmp_dir("cvt");
    let cs = dir.join("A.cs");
    fs::write(&cs, "using System;\npublic class A { public int F(int x) { return x; } }\n").unwrap();
    let out = convert_file(&cs, None).unwrap();
    assert_eq!(out.extension().unwrap(), "rt");
    let body = fs::read_to_string(&out).unwrap();
    assert!(body.contains("export class") || body.contains("class A"));
}
