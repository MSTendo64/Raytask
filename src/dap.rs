//! Debug Adapter Protocol (stdio) for RayTask VM debugging.

use crate::debug_io::{self, DebugOutputKind};
use crate::value::Value;
use crate::vm::{DebugBreakpoint, Vm};
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    Continue,
    /// Step over (same or shallower frame, different line).
    Next,
    StepIn,
    StepOut,
    Pause,
}

pub use StepMode as VmStepMode;

#[derive(Debug)]
enum DapCommand {
    SetBreakpoints {
        path: String,
        bps: Vec<DebugBreakpoint>,
        request_seq: i64,
    },
    Continue { request_seq: i64 },
    Next { request_seq: i64 },
    StepIn { request_seq: i64 },
    StepOut { request_seq: i64 },
    Pause { request_seq: i64 },
    StackTrace { request_seq: i64 },
    Scopes { request_seq: i64, frame_id: i64 },
    Variables { request_seq: i64, variables_ref: i64 },
    Evaluate { request_seq: i64, expression: String },
    Threads { request_seq: i64 },
    Restart { request_seq: i64 },
    Disconnect,
    ConfigurationDone,
}

struct StdoutWriter;

impl StdoutWriter {
    fn send(&mut self, seq: &mut i64, body: Json) {
        *seq += 1;
        let mut msg = body;
        if let Some(obj) = msg.as_object_mut() {
            obj.insert("seq".into(), json!(*seq));
        }
        let data = serde_json::to_string(&msg).unwrap_or_else(|_| "{}".into());
        let header = format!("Content-Length: {}\r\n\r\n", data.len());
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(header.as_bytes());
        let _ = stdout.write_all(data.as_bytes());
        let _ = stdout.flush();
    }
}

fn normalize_path(p: &str) -> String {
    let s = p.replace('\\', "/");
    #[cfg(windows)]
    {
        s.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        s
    }
}

/// Entry: `raytask dap` — speak DAP over stdin/stdout.
pub fn run_dap() -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        loop {
            match read_message(&mut handle) {
                Ok(Some(msg)) => {
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
    });

    let mut out = StdoutWriter;
    let mut seq = 0i64;
    let mut program: Option<PathBuf> = None;
    let mut stop_on_entry = true;
    let mut no_typecheck = false;
    let mut gc = true;
    let mut pending_bps: HashMap<String, Vec<DebugBreakpoint>> = HashMap::new();

    loop {
        let msg = rx.recv()?;
        let v: Json = serde_json::from_str(&msg)?;
        let cmd = v.get("command").and_then(|c| c.as_str()).unwrap_or("");
        let req_seq = v.get("seq").and_then(|s| s.as_i64()).unwrap_or(0);
        let args = v.get("arguments").cloned().unwrap_or(json!({}));

        match cmd {
            "initialize" => {
                out.send(
                    &mut seq,
                    json!({
                        "type": "response",
                        "request_seq": req_seq,
                        "success": true,
                        "command": "initialize",
                        "body": {
                            "supportsConfigurationDoneRequest": true,
                            "supportsEvaluateForHovers": true,
                            "supportsStepInTargetsRequest": false,
                            "supportsRestartRequest": true,
                            "supportsConditionalBreakpoints": true,
                            "supportsLogPoints": true,
                            "supportsSetVariable": false,
                            "exceptionBreakpointFilters": [
                                { "filter": "all", "label": "All Exceptions", "default": true }
                            ]
                        }
                    }),
                );
                out.send(
                    &mut seq,
                    json!({
                        "type": "event",
                        "event": "initialized",
                        "body": {}
                    }),
                );
            }
            "launch" => {
                if let Some(p) = args.get("program").and_then(|x| x.as_str()) {
                    program = Some(PathBuf::from(p));
                }
                if let Some(c) = args.get("cwd").and_then(|x| x.as_str()) {
                    let _ = std::env::set_current_dir(c);
                }
                stop_on_entry = args
                    .get("stopOnEntry")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(true);
                no_typecheck = args
                    .get("noTypecheck")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                gc = args.get("gc").and_then(|x| x.as_bool()).unwrap_or(true);
                if let Some(arr) = args.get("args").and_then(|a| a.as_array()) {
                    let joined: Vec<String> = arr
                        .iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect();
                    std::env::set_var("RAYTASK_ARGS", joined.join("\x1f"));
                }
                out.send(
                    &mut seq,
                    json!({
                        "type": "response",
                        "request_seq": req_seq,
                        "success": true,
                        "command": "launch"
                    }),
                );
            }
            "setBreakpoints" => {
                let path = args
                    .get("source")
                    .and_then(|s| s.get("path"))
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string();
                let bps = extract_breakpoints(&args);
                let key = normalize_path(&path);
                let body_bps: Vec<Json> = bps
                    .iter()
                    .map(|b| {
                        json!({
                            "verified": true,
                            "line": b.line
                        })
                    })
                    .collect();
                pending_bps.insert(key, bps);
                out.send(
                    &mut seq,
                    json!({
                        "type": "response",
                        "request_seq": req_seq,
                        "success": true,
                        "command": "setBreakpoints",
                        "body": { "breakpoints": body_bps }
                    }),
                );
            }
            "setExceptionBreakpoints" => {
                out.send(
                    &mut seq,
                    json!({
                        "type": "response",
                        "request_seq": req_seq,
                        "success": true,
                        "command": "setExceptionBreakpoints",
                        "body": {}
                    }),
                );
            }
            "configurationDone" => {
                out.send(
                    &mut seq,
                    json!({
                        "type": "response",
                        "request_seq": req_seq,
                        "success": true,
                        "command": "configurationDone"
                    }),
                );
                break;
            }
            "disconnect" | "terminate" => {
                out.send(
                    &mut seq,
                    json!({
                        "type": "response",
                        "request_seq": req_seq,
                        "success": true,
                        "command": cmd
                    }),
                );
                return Ok(());
            }
            "threads" => {
                out.send(
                    &mut seq,
                    json!({
                        "type": "response",
                        "request_seq": req_seq,
                        "success": true,
                        "command": "threads",
                        "body": { "threads": [{ "id": 1, "name": "main" }] }
                    }),
                );
            }
            other => {
                out.send(
                    &mut seq,
                    json!({
                        "type": "response",
                        "request_seq": req_seq,
                        "success": true,
                        "command": other
                    }),
                );
            }
        }
    }

    let program = program.ok_or("DAP launch missing program")?;
    let (cmd_tx, cmd_rx) = mpsc::channel::<DapCommand>();
    let pause_flag = Arc::new(AtomicBool::new(false));
    let pause_for_reader = pause_flag.clone();

    thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            let Ok(v) = serde_json::from_str::<Json>(&msg) else {
                continue;
            };
            let cmd = v.get("command").and_then(|c| c.as_str()).unwrap_or("");
            let req_seq = v.get("seq").and_then(|s| s.as_i64()).unwrap_or(0);
            let args = v.get("arguments").cloned().unwrap_or(json!({}));
            let mapped = match cmd {
                "continue" => Some(DapCommand::Continue {
                    request_seq: req_seq,
                }),
                "next" => Some(DapCommand::Next {
                    request_seq: req_seq,
                }),
                "stepIn" => Some(DapCommand::StepIn {
                    request_seq: req_seq,
                }),
                "stepOut" => Some(DapCommand::StepOut {
                    request_seq: req_seq,
                }),
                "pause" => {
                    pause_for_reader.store(true, Ordering::SeqCst);
                    Some(DapCommand::Pause {
                        request_seq: req_seq,
                    })
                }
                "disconnect" | "terminate" => Some(DapCommand::Disconnect),
                "restart" => Some(DapCommand::Restart {
                    request_seq: req_seq,
                }),
                "configurationDone" => Some(DapCommand::ConfigurationDone),
                "setBreakpoints" => {
                    let path = args
                        .get("source")
                        .and_then(|s| s.get("path"))
                        .and_then(|p| p.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(DapCommand::SetBreakpoints {
                        path,
                        bps: extract_breakpoints(&args),
                        request_seq: req_seq,
                    })
                }
                "stackTrace" => Some(DapCommand::StackTrace {
                    request_seq: req_seq,
                }),
                "scopes" => Some(DapCommand::Scopes {
                    request_seq: req_seq,
                    frame_id: args.get("frameId").and_then(|x| x.as_i64()).unwrap_or(0),
                }),
                "variables" => Some(DapCommand::Variables {
                    request_seq: req_seq,
                    variables_ref: args
                        .get("variablesReference")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(1),
                }),
                "evaluate" => Some(DapCommand::Evaluate {
                    request_seq: req_seq,
                    expression: args
                        .get("expression")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                }),
                "threads" => Some(DapCommand::Threads {
                    request_seq: req_seq,
                }),
                _ => None,
            };
            if let Some(m) = mapped {
                if cmd_tx.send(m).is_err() {
                    break;
                }
            }
            if cmd == "disconnect" || cmd == "terminate" {
                break;
            }
        }
    });

    loop {
        match run_debug_session(
            &program,
            stop_on_entry,
            no_typecheck,
            gc,
            pending_bps.clone(),
            &cmd_rx,
            &mut out,
            &mut seq,
            pause_flag.clone(),
        )? {
            SessionEnd::Done => return Ok(()),
            SessionEnd::Restart => {
                // clear pause and re-run
                pause_flag.store(false, Ordering::SeqCst);
                continue;
            }
        }
    }
}

fn extract_breakpoints(args: &Json) -> Vec<DebugBreakpoint> {
    args.get("breakpoints")
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|bp| {
                    let line = bp.get("line").and_then(|l| l.as_u64()).map(|n| n as usize)?;
                    Some(DebugBreakpoint {
                        line,
                        condition: bp
                            .get("condition")
                            .and_then(|c| c.as_str())
                            .map(|s| s.to_string()),
                        log_message: bp
                            .get("logMessage")
                            .and_then(|c| c.as_str())
                            .map(|s| s.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            content_length = Some(rest.trim().parse::<usize>().unwrap_or(0));
        }
    }
    let len = content_length.unwrap_or(0);
    if len == 0 {
        return Ok(None);
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

enum SessionEnd {
    Done,
    Restart,
}

fn run_debug_session(
    program: &Path,
    stop_on_entry: bool,
    no_typecheck: bool,
    gc: bool,
    mut breakpoints: HashMap<String, Vec<DebugBreakpoint>>,
    cmd_rx: &Receiver<DapCommand>,
    out: &mut StdoutWriter,
    seq: &mut i64,
    pause_flag: Arc<AtomicBool>,
) -> Result<SessionEnd, Box<dyn std::error::Error>> {
    let (out_tx, out_rx): (
        Sender<(DebugOutputKind, String)>,
        Receiver<(DebugOutputKind, String)>,
    ) = mpsc::channel();
    debug_io::install(out_tx);

    let opts = crate::RunOptions {
        gc,
        gc_stress: false,
        no_typecheck,
        no_stdlib: false,
    };

    let module = if program
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("rtbc"))
        .unwrap_or(false)
    {
        let bytes = std::fs::read(program)?;
        let mut m = crate::bytecode_format::deserialize_module(&bytes)?;
        if let Some(sym_path) = crate::debug_symbols::find_sidecar(program) {
            match crate::debug_symbols::DebugSymbols::read_file(&sym_path) {
                Ok(sym) => {
                    sym.apply_to_module(&mut m);
                    out.send(
                        seq,
                        json!({
                            "type": "event",
                            "event": "output",
                            "body": {
                                "category": "console",
                                "output": format!("Loaded debug symbols: {}\n", sym_path.display())
                            }
                        }),
                    );
                }
                Err(e) => {
                    out.send(
                        seq,
                        json!({
                            "type": "event",
                            "event": "output",
                            "body": {
                                "category": "stderr",
                                "output": format!("Failed to load {}: {}\n", sym_path.display(), e)
                            }
                        }),
                    );
                }
            }
        }
        m
    } else {
        let source = std::fs::read_to_string(program)?;
        let program_ast = crate::resolve::resolve_program(&source, Some(program))?;
        let program_ast = if opts.no_typecheck {
            program_ast
        } else {
            crate::sema::typecheck_or_err(&program_ast)?;
            program_ast
        };
        let program_ast = crate::mono::monomorphize(program_ast);
        let mut m = crate::compiler::Compiler::new()
            .with_source(program.display().to_string())
            .compile(&program_ast)?;
        crate::debug_symbols::stamp_source(&mut m, program);
        m
    };
    crate::ffi::prepare_module_ffi(&module.ffi, program.parent())?;

    let mut vm = Vm::with_gc(
        module,
        crate::gc::GcConfig {
            enabled: opts.gc,
            threshold_bytes: 256 * 1024,
            stress: false,
        },
    );

    // Ensure launch file key exists for breakpoints set against it
    let launch_key = normalize_path(&program.display().to_string());
    if !breakpoints.contains_key(&launch_key) {
        // also try absolute
        if let Ok(abs) = std::fs::canonicalize(program) {
            let k = normalize_path(&abs.display().to_string());
            if let Some(v) = breakpoints.remove(&k) {
                breakpoints.insert(launch_key.clone(), v);
            }
        }
    }

    vm.debug_begin(program.to_path_buf(), breakpoints.clone(), pause_flag.clone());

    let flush_output = |out: &mut StdoutWriter, seq: &mut i64, out_rx: &Receiver<_>| {
        while let Ok((kind, text)) = out_rx.try_recv() {
            let category = match kind {
                DebugOutputKind::Stdout => "stdout",
                DebugOutputKind::Stderr => "stderr",
            };
            out.send(
                seq,
                json!({
                    "type": "event",
                    "event": "output",
                    "body": { "category": category, "output": text }
                }),
            );
        }
    };

    let mut mode = if stop_on_entry {
        vm.debug_mark_current_line();
        out.send(
            seq,
            json!({
                "type": "event",
                "event": "stopped",
                "body": {
                    "reason": "entry",
                    "threadId": 1,
                    "allThreadsStopped": true
                }
            }),
        );
        StepMode::Pause
    } else {
        StepMode::Continue
    };

    let mut scopes_frame: i64 = 0;

    loop {
        flush_output(out, seq, &out_rx);

        while mode == StepMode::Pause {
            flush_output(out, seq, &out_rx);
            match cmd_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(DapCommand::Continue { request_seq }) => {
                    mode = StepMode::Continue;
                    respond_ok(out, seq, request_seq, "continue");
                }
                Ok(DapCommand::Next { request_seq }) => {
                    mode = StepMode::Next;
                    respond_ok(out, seq, request_seq, "next");
                }
                Ok(DapCommand::StepIn { request_seq }) => {
                    mode = StepMode::StepIn;
                    respond_ok(out, seq, request_seq, "stepIn");
                }
                Ok(DapCommand::StepOut { request_seq }) => {
                    mode = StepMode::StepOut;
                    respond_ok(out, seq, request_seq, "stepOut");
                }
                Ok(DapCommand::Pause { request_seq }) => {
                    mode = StepMode::Pause;
                    respond_ok(out, seq, request_seq, "pause");
                }
                Ok(DapCommand::SetBreakpoints {
                    path,
                    bps,
                    request_seq,
                }) => {
                    let key = normalize_path(&path);
                    let body: Vec<Json> = bps
                        .iter()
                        .map(|b| json!({ "verified": true, "line": b.line }))
                        .collect();
                    breakpoints.insert(key, bps);
                    vm.debug_set_breakpoints(breakpoints.clone());
                    out.send(
                        seq,
                        json!({
                            "type": "response",
                            "request_seq": request_seq,
                            "success": true,
                            "command": "setBreakpoints",
                            "body": { "breakpoints": body }
                        }),
                    );
                }
                Ok(DapCommand::StackTrace { request_seq }) => {
                    let frames = vm.debug_stack_frames();
                    out.send(
                        seq,
                        json!({
                            "type": "response",
                            "request_seq": request_seq,
                            "success": true,
                            "command": "stackTrace",
                            "body": {
                                "stackFrames": frames,
                                "totalFrames": frames.len()
                            }
                        }),
                    );
                }
                Ok(DapCommand::Scopes {
                    request_seq,
                    frame_id,
                }) => {
                    scopes_frame = frame_id;
                    out.send(
                        seq,
                        json!({
                            "type": "response",
                            "request_seq": request_seq,
                            "success": true,
                            "command": "scopes",
                            "body": {
                                "scopes": [{
                                    "name": "Locals",
                                    "variablesReference": 1,
                                    "expensive": false
                                }, {
                                    "name": "Globals",
                                    "variablesReference": 2,
                                    "expensive": false
                                }]
                            }
                        }),
                    );
                }
                Ok(DapCommand::Variables {
                    request_seq,
                    variables_ref,
                }) => {
                    let vars = if variables_ref == 1 {
                        vm.debug_locals_for_frame(scopes_frame as usize)
                    } else if variables_ref == 2 {
                        vm.debug_globals()
                    } else {
                        vm.debug_expand_var(variables_ref)
                    };
                    out.send(
                        seq,
                        json!({
                            "type": "response",
                            "request_seq": request_seq,
                            "success": true,
                            "command": "variables",
                            "body": { "variables": vars }
                        }),
                    );
                }
                Ok(DapCommand::Evaluate {
                    request_seq,
                    expression,
                }) => {
                    let expr = expression.trim();
                    let val = vm.debug_eval_name(expr);
                    out.send(
                        seq,
                        json!({
                            "type": "response",
                            "request_seq": request_seq,
                            "success": true,
                            "command": "evaluate",
                            "body": {
                                "result": val,
                                "variablesReference": 0
                            }
                        }),
                    );
                }
                Ok(DapCommand::Threads { request_seq }) => {
                    out.send(
                        seq,
                        json!({
                            "type": "response",
                            "request_seq": request_seq,
                            "success": true,
                            "command": "threads",
                            "body": { "threads": [{ "id": 1, "name": "main" }] }
                        }),
                    );
                }
                Ok(DapCommand::Restart { request_seq }) => {
                    respond_ok(out, seq, request_seq, "restart");
                    debug_io::uninstall();
                    flush_output(out, seq, &out_rx);
                    return Ok(SessionEnd::Restart);
                }
                Ok(DapCommand::Disconnect) => {
                    out.send(
                        seq,
                        json!({
                            "type": "event",
                            "event": "terminated",
                            "body": {}
                        }),
                    );
                    debug_io::uninstall();
                    return Ok(SessionEnd::Done);
                }
                Ok(DapCommand::ConfigurationDone) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => {
                    debug_io::uninstall();
                    return Ok(SessionEnd::Done);
                }
            }
        }

        // Drain non-blocking commands while about to run (e.g. pause already set)
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                DapCommand::Pause { request_seq } => {
                    respond_ok(out, seq, request_seq, "pause");
                    pause_flag.store(true, Ordering::SeqCst);
                }
                DapCommand::SetBreakpoints { path, bps, .. } => {
                    breakpoints.insert(normalize_path(&path), bps);
                    vm.debug_set_breakpoints(breakpoints.clone());
                }
                DapCommand::Disconnect => {
                    debug_io::uninstall();
                    return Ok(SessionEnd::Done);
                }
                DapCommand::Restart { request_seq } => {
                    respond_ok(out, seq, request_seq, "restart");
                    debug_io::uninstall();
                    return Ok(SessionEnd::Restart);
                }
                _ => {}
            }
        }

        match vm.debug_run(mode)? {
            DebugStop::Breakpoint { line: _ } => {
                mode = StepMode::Pause;
                flush_output(out, seq, &out_rx);
                out.send(
                    seq,
                    json!({
                        "type": "event",
                        "event": "stopped",
                        "body": {
                            "reason": "breakpoint",
                            "threadId": 1,
                            "allThreadsStopped": true
                        }
                    }),
                );
            }
            DebugStop::Step { line: _ } => {
                mode = StepMode::Pause;
                flush_output(out, seq, &out_rx);
                out.send(
                    seq,
                    json!({
                        "type": "event",
                        "event": "stopped",
                        "body": {
                            "reason": "step",
                            "threadId": 1,
                            "allThreadsStopped": true
                        }
                    }),
                );
            }
            DebugStop::Pause { line: _ } => {
                mode = StepMode::Pause;
                flush_output(out, seq, &out_rx);
                out.send(
                    seq,
                    json!({
                        "type": "event",
                        "event": "stopped",
                        "body": {
                            "reason": "pause",
                            "threadId": 1,
                            "allThreadsStopped": true
                        }
                    }),
                );
            }
            DebugStop::Terminated { result } => {
                flush_output(out, seq, &out_rx);
                out.send(
                    seq,
                    json!({
                        "type": "event",
                        "event": "output",
                        "body": {
                            "category": "console",
                            "output": format!("Program finished: {}\n", result)
                        }
                    }),
                );
                out.send(
                    seq,
                    json!({
                        "type": "event",
                        "event": "terminated",
                        "body": {}
                    }),
                );
                out.send(
                    seq,
                    json!({
                        "type": "event",
                        "event": "exited",
                        "body": { "exitCode": 0 }
                    }),
                );
                debug_io::uninstall();
                return Ok(SessionEnd::Done);
            }
            DebugStop::Error(e) => {
                flush_output(out, seq, &out_rx);
                out.send(
                    seq,
                    json!({
                        "type": "event",
                        "event": "output",
                        "body": {
                            "category": "stderr",
                            "output": format!("Runtime error: {}\n", e)
                        }
                    }),
                );
                out.send(
                    seq,
                    json!({
                        "type": "event",
                        "event": "stopped",
                        "body": {
                            "reason": "exception",
                            "description": format!("{}", e),
                            "threadId": 1,
                            "allThreadsStopped": true
                        }
                    }),
                );
                mode = StepMode::Pause;
            }
        }
    }
}

fn respond_ok(out: &mut StdoutWriter, seq: &mut i64, request_seq: i64, command: &str) {
    let body = if command == "continue" {
        json!({ "allThreadsContinued": true })
    } else {
        json!({})
    };
    out.send(
        seq,
        json!({
            "type": "response",
            "request_seq": request_seq,
            "success": true,
            "command": command,
            "body": body
        }),
    );
}

#[derive(Debug)]
pub enum DebugStop {
    Breakpoint { line: usize },
    Step { line: usize },
    Pause { line: usize },
    Terminated { result: Value },
    Error(crate::error::RuntimeError),
}
