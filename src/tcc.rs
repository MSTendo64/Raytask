use std::cell::RefCell;
use std::env;
use std::ffi::{c_char, c_int, c_void, CString};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[allow(non_camel_case_types)]
type TCCState = c_void;

const TCC_OUTPUT_MEMORY: c_int = 1;
const TCC_OUTPUT_EXE: c_int = 2;
const TCC_OUTPUT_OBJ: c_int = 3;
const TCC_OUTPUT_DLL: c_int = 4;

unsafe extern "C" {
    fn tcc_new() -> *mut TCCState;
    fn tcc_delete(s: *mut TCCState);
    fn tcc_set_lib_path(s: *mut TCCState, path: *const c_char);
    fn tcc_set_error_func(s: *mut TCCState, error_opaque: *mut c_void, error_func: TCCErrorFunc);
    fn tcc_set_options(s: *mut TCCState, options: *const c_char) -> c_int;
    fn tcc_add_include_path(s: *mut TCCState, pathname: *const c_char) -> c_int;
    fn tcc_add_sysinclude_path(s: *mut TCCState, pathname: *const c_char) -> c_int;
    fn tcc_add_library_path(s: *mut TCCState, pathname: *const c_char) -> c_int;
    fn tcc_add_library(s: *mut TCCState, libraryname: *const c_char) -> c_int;
    fn tcc_add_file(s: *mut TCCState, filename: *const c_char) -> c_int;
    fn tcc_compile_string(s: *mut TCCState, buf: *const c_char) -> c_int;
    fn tcc_set_output_type(s: *mut TCCState, output_type: c_int) -> c_int;
    fn tcc_output_file(s: *mut TCCState, filename: *const c_char) -> c_int;
    fn tcc_run(s: *mut TCCState, argc: c_int, argv: *mut *mut c_char) -> c_int;
    fn tcc_relocate(s: *mut TCCState) -> c_int;
    fn tcc_get_symbol(s: *mut TCCState, name: *const c_char) -> *mut c_void;
}

type TCCErrorFunc = Option<unsafe extern "C" fn(opaque: *mut c_void, msg: *const c_char)>;

thread_local! {
    static LAST_ERRORS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

static BOOTSTRAP_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy)]
pub enum OutputKind {
    Exe,
    Dll,
    Obj,
    Memory,
}

pub struct TinyCc {
    state: *mut TCCState,
}

impl TinyCc {
    pub fn new() -> Result<Self, String> {
        ensure_runtime_ready()?;
        clear_errors();
        let state = unsafe { tcc_new() };
        if state.is_null() {
            return Err("tcc_new failed".into());
        }
        let me = Self { state };
        unsafe {
            tcc_set_error_func(me.state, std::ptr::null_mut(), Some(error_callback));
        }
        me.init_default_paths()?;
        Ok(me)
    }

    fn init_default_paths(&self) -> Result<(), String> {
        let root = runtime_root();
        self.set_lib_path(&root)?;
        self.add_include_path(&root.join("include"))?;
        self.add_sysinclude_path(&root.join("include"))?;
        self.add_library_path(&root.join("lib"))?;
        // Also expose upstream include/ when runtime staging is incomplete.
        let vendored_include = vendored_root().join("include");
        if vendored_include.exists() {
            let _ = self.add_include_path(&vendored_include);
        }
        if cfg!(windows) {
            let win_include = vendored_root().join("win32").join("include");
            if win_include.exists() {
                let _ = self.add_sysinclude_path(&win_include);
            }
            let win_lib = vendored_root().join("win32").join("lib");
            if win_lib.exists() {
                let _ = self.add_library_path(&win_lib);
            }
        }
        Ok(())
    }

    pub fn set_lib_path(&self, path: &Path) -> Result<(), String> {
        let path = to_cstring(path)?;
        unsafe { tcc_set_lib_path(self.state, path.as_ptr()) };
        Ok(())
    }

    pub fn set_options(&self, options: &str) -> Result<(), String> {
        let options = CString::new(options).map_err(|e| e.to_string())?;
        clear_errors();
        let rc = unsafe { tcc_set_options(self.state, options.as_ptr()) };
        if rc < 0 {
            return Err(take_errors_or(format!(
                "tcc_set_options failed: {}",
                options.to_string_lossy()
            )));
        }
        Ok(())
    }

    pub fn add_include_path(&self, path: &Path) -> Result<(), String> {
        let path = to_cstring(path)?;
        clear_errors();
        if unsafe { tcc_add_include_path(self.state, path.as_ptr()) } < 0 {
            return Err(take_errors_or(format!(
                "tcc_add_include_path failed: {}",
                path.to_string_lossy()
            )));
        }
        Ok(())
    }

    pub fn add_sysinclude_path(&self, path: &Path) -> Result<(), String> {
        let path = to_cstring(path)?;
        clear_errors();
        if unsafe { tcc_add_sysinclude_path(self.state, path.as_ptr()) } < 0 {
            return Err(take_errors_or(format!(
                "tcc_add_sysinclude_path failed: {}",
                path.to_string_lossy()
            )));
        }
        Ok(())
    }

    pub fn add_library_path(&self, path: &Path) -> Result<(), String> {
        let path = to_cstring(path)?;
        clear_errors();
        if unsafe { tcc_add_library_path(self.state, path.as_ptr()) } < 0 {
            return Err(take_errors_or(format!(
                "tcc_add_library_path failed: {}",
                path.to_string_lossy()
            )));
        }
        Ok(())
    }

    pub fn add_library(&self, name: &str) -> Result<(), String> {
        let name = CString::new(name).map_err(|e| e.to_string())?;
        clear_errors();
        if unsafe { tcc_add_library(self.state, name.as_ptr()) } < 0 {
            return Err(take_errors_or(format!(
                "tcc_add_library failed: {}",
                name.to_string_lossy()
            )));
        }
        Ok(())
    }

    pub fn add_file(&self, file: &Path) -> Result<(), String> {
        let file = to_cstring(file)?;
        clear_errors();
        if unsafe { tcc_add_file(self.state, file.as_ptr()) } < 0 {
            return Err(take_errors_or(format!(
                "tcc_add_file failed: {}",
                file.to_string_lossy()
            )));
        }
        Ok(())
    }

    pub fn compile_string(&self, source: &str) -> Result<(), String> {
        let source = CString::new(source).map_err(|e| e.to_string())?;
        clear_errors();
        if unsafe { tcc_compile_string(self.state, source.as_ptr()) } < 0 {
            return Err(take_errors_or("tcc_compile_string failed".into()));
        }
        Ok(())
    }

    pub fn set_output_kind(&self, kind: OutputKind) -> Result<(), String> {
        let ty = match kind {
            OutputKind::Memory => TCC_OUTPUT_MEMORY,
            OutputKind::Exe => TCC_OUTPUT_EXE,
            OutputKind::Obj => TCC_OUTPUT_OBJ,
            OutputKind::Dll => TCC_OUTPUT_DLL,
        };
        clear_errors();
        if unsafe { tcc_set_output_type(self.state, ty) } < 0 {
            return Err(take_errors_or("tcc_set_output_type failed".into()));
        }
        Ok(())
    }

    pub fn output_file(&self, output: &Path) -> Result<(), String> {
        let output = to_cstring(output)?;
        clear_errors();
        if unsafe { tcc_output_file(self.state, output.as_ptr()) } < 0 {
            return Err(take_errors_or(format!(
                "tcc_output_file failed: {}",
                output.to_string_lossy()
            )));
        }
        Ok(())
    }

    pub fn run(&self, argv: &[String]) -> Result<i32, String> {
        let cstrings = argv
            .iter()
            .map(|s| CString::new(s.as_str()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ptrs = cstrings
            .iter()
            .map(|s| s.as_ptr() as *mut c_char)
            .collect::<Vec<_>>();
        clear_errors();
        let rc = unsafe { tcc_run(self.state, ptrs.len() as c_int, ptrs.as_mut_ptr()) };
        if !take_errors().is_empty() && rc != 0 {
            return Err(take_errors_or(format!("tcc_run failed with exit code {rc}")));
        }
        Ok(rc)
    }

    pub fn relocate(&self) -> Result<(), String> {
        clear_errors();
        if unsafe { tcc_relocate(self.state) } < 0 {
            return Err(take_errors_or("tcc_relocate failed".into()));
        }
        Ok(())
    }

    pub fn get_symbol(&self, name: &str) -> Result<*mut c_void, String> {
        let name = CString::new(name).map_err(|e| e.to_string())?;
        let ptr = unsafe { tcc_get_symbol(self.state, name.as_ptr()) };
        if ptr.is_null() {
            return Err(format!("symbol not found: {}", name.to_string_lossy()));
        }
        Ok(ptr)
    }
}

impl Drop for TinyCc {
    fn drop(&mut self) {
        unsafe { tcc_delete(self.state) };
    }
}

pub fn compile_c_to_path(
    input: &Path,
    output: &Path,
    kind: OutputKind,
    debug: bool,
    link_libs: &[String],
    include_dirs: &[PathBuf],
) -> Result<(), String> {
    let tcc = TinyCc::new()?;
    tcc.set_output_kind(kind)?;
    // On Windows, TCC does not export DLL symbols by default — tell it to
    // export all non-local symbols so that GetProcAddress / dlopen can find them.
    if matches!(kind, OutputKind::Dll) && cfg!(windows) {
        tcc.set_options("-Wl,--export-all-symbols")?;
    }
    if debug {
        let _ = tcc.set_options("-g");
        let _ = tcc.set_options("-O0");
    } else {
        let _ = tcc.set_options("-O2");
    }
    // Use add_include_path with the raw (non-canonicalized) path.
    // canonicalize() adds \\?\ prefix on Windows which TCC can't handle.
    for inc in include_dirs {
        let _ = tcc.add_include_path(inc);
    }
    for lib in link_libs {
        if lib.ends_with(".c") || lib.ends_with(".o") || lib.ends_with(".obj") {
            tcc.add_file(Path::new(lib))?;
        } else if lib.ends_with(".a")
            || lib.ends_with(".so")
            || lib.ends_with(".dll")
            || lib.ends_with(".dylib")
            || lib.ends_with(".lib")
        {
            tcc.add_file(Path::new(lib))?;
        } else if let Some(name) = lib.strip_prefix("lib") {
            tcc.add_library(name)?;
        } else {
            tcc.add_library(lib.trim_end_matches(".dll"))?;
        }
    }
    tcc.add_file(input)?;
    tcc.output_file(output)
}

pub fn run_tcc_cli(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err(
            "usage: raytask tcc [options] file.c [-o out] | raytask tcc -run file.c [-- args...]"
                .into(),
        );
    }

    let mut output: Option<PathBuf> = None;
    let mut kind = OutputKind::Exe;
    let mut run_mode = false;
    let mut files = Vec::new();
    let mut runtime_argv = vec!["tcc".to_string()];
    let mut options = Vec::new();
    let mut after_double_dash = false;
    let mut i = 0usize;
    while i < args.len() {
        let arg = &args[i];
        if after_double_dash {
            runtime_argv.push(arg.clone());
            i += 1;
            continue;
        }
        match arg.as_str() {
            "--" => {
                after_double_dash = true;
            }
            "-o" => {
                i += 1;
                let next = args
                    .get(i)
                    .ok_or_else(|| "missing value after -o".to_string())?;
                output = Some(PathBuf::from(next));
            }
            "-c" => {
                kind = OutputKind::Obj;
            }
            "-shared" => {
                kind = OutputKind::Dll;
            }
            "-run" => {
                run_mode = true;
                kind = OutputKind::Memory;
            }
            _ if arg.ends_with(".c")
                || arg.ends_with(".h")
                || arg.ends_with(".o")
                || arg.ends_with(".obj")
                || arg.ends_with(".a")
                || arg.ends_with(".so")
                || arg.ends_with(".dll")
                || arg.ends_with(".dylib")
                || arg.ends_with(".S")
                || arg.ends_with(".s") =>
            {
                if run_mode && !files.is_empty() {
                    runtime_argv.push(arg.clone());
                } else {
                    files.push(PathBuf::from(arg));
                }
            }
            _ => {
                if run_mode && !files.is_empty() {
                    runtime_argv.push(arg.clone());
                } else {
                    options.push(arg.clone());
                }
            }
        }
        i += 1;
    }

    if files.is_empty() {
        return Err("no input files".into());
    }

    let tcc = TinyCc::new()?;
    tcc.set_output_kind(kind)?;
    for opt in &options {
        tcc.set_options(opt)?;
    }
    for file in &files {
        tcc.add_file(file)?;
    }
    if run_mode {
        let rc = tcc.run(&runtime_argv)?;
        if rc != 0 {
            return Err(format!("tcc run failed with exit code {rc}"));
        }
        return Ok(());
    }
    let out = output.unwrap_or_else(|| default_output_path(&files, kind));
    tcc.output_file(&out)?;
    println!("{}", out.display());
    Ok(())
}

fn default_output_path(files: &[PathBuf], kind: OutputKind) -> PathBuf {
    let first = files
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("a"));
    match kind {
        OutputKind::Obj => first.with_extension("o"),
        OutputKind::Dll => {
            if cfg!(windows) {
                first.with_extension("dll")
            } else {
                first.with_extension("so")
            }
        }
        OutputKind::Exe | OutputKind::Memory => {
            if cfg!(windows) {
                first.with_extension("exe")
            } else {
                first.with_extension("out")
            }
        }
    }
}

fn ensure_runtime_ready() -> Result<(), String> {
    let _guard = BOOTSTRAP_LOCK
        .lock()
        .map_err(|_| "tcc runtime bootstrap lock poisoned".to_string())?;
    let root = runtime_root();
    let libtcc1 = root.join("lib").join("libtcc1.a");
    if libtcc1.exists() {
        return Ok(());
    }
    // Runtime should normally be staged by build.rs. Missing archive is a soft warning for
    // memory/-run mode, but EXE output usually needs it.
    if root.join("include").exists() {
        return Ok(());
    }
    Err(format!(
        "TCC runtime not found at {} (rebuild RayTask to stage vendored TinyCC runtime)",
        root.display()
    ))
}

fn vendored_root() -> PathBuf {
    if let Ok(path) = env::var("RAYTASK_VENDORED_TCC_ROOT") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("RAYTASK_VENDORED_TCC_ROOT"))
}

fn runtime_root() -> PathBuf {
    if let Ok(path) = env::var("RAYTASK_TCC_RUNTIME") {
        return PathBuf::from(path);
    }
    if let Ok(path) = env::var("RAYTASK_VENDORED_TCC_ROOT") {
        if cfg!(windows) {
            return PathBuf::from(path).join("win32");
        }
        return PathBuf::from(path);
    }
    PathBuf::from(env!("RAYTASK_TCC_RUNTIME"))
}

fn to_cstring(path: &Path) -> Result<CString, String> {
    CString::new(path_to_utf8(path)).map_err(|e| e.to_string())
}

fn path_to_utf8(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn clear_errors() {
    LAST_ERRORS.with(|cell| cell.borrow_mut().clear());
}

fn take_errors() -> Vec<String> {
    LAST_ERRORS.with(|cell| std::mem::take(&mut *cell.borrow_mut()))
}

fn take_errors_or(fallback: String) -> String {
    let errs = take_errors();
    if errs.is_empty() {
        fallback
    } else {
        errs.join("\n")
    }
}

unsafe extern "C" fn error_callback(_opaque: *mut c_void, msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(msg) }
        .to_string_lossy()
        .into_owned();
    LAST_ERRORS.with(|cell| cell.borrow_mut().push(text));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendored_tcc_compiles_and_runs_in_memory() {
        let tcc = TinyCc::new().expect("TinyCc::new");
        tcc.set_output_kind(OutputKind::Memory).expect("output type");
        tcc.compile_string(
            r#"
            int add(int a, int b) { return a + b; }
            int main(void) { return add(40, 2); }
            "#,
        )
        .expect("compile_string");
        let rc = tcc.run(&["tcc".into()]).expect("run");
        assert_eq!(rc, 42);
    }

    #[test]
    fn vendored_tcc_exposes_symbols_after_relocate() {
        let tcc = TinyCc::new().expect("TinyCc::new");
        tcc.set_output_kind(OutputKind::Memory).expect("output type");
        tcc.compile_string("int raytask_tcc_mul(int a, int b) { return a * b; }")
            .expect("compile_string");
        tcc.relocate().expect("relocate");
        let sym = tcc.get_symbol("raytask_tcc_mul").expect("symbol");
        let f: unsafe extern "C" fn(c_int, c_int) -> c_int =
            unsafe { std::mem::transmute(sym) };
        let value = unsafe { f(6, 7) };
        assert_eq!(value, 42);
    }
}
