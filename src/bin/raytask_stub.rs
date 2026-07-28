//! RayTask runtime stub — standalone executable that runs bytecode
//! appended to itself (or falls back to a .rtbc path argument).

use raytask::bytecode_format::extract_app_payload;
use raytask::run_bytecode;
use std::env;
use std::fs;
use std::process;

fn main() {
    // 1) Prefer bytecode embedded in this executable
    if let Ok(exe) = env::current_exe() {
        if let Ok(data) = fs::read(&exe) {
            if let Some(payload) = extract_app_payload(&data) {
                if let Err(e) = run_bytecode(&payload) {
                    eprintln!("raytask: {e}");
                    process::exit(1);
                }
                return;
            }
        }
    }

    // 2) CLI: raytask-stub program.rtbc
    let mut args = env::args().skip(1);
    if let Some(path) = args.next() {
        match fs::read(&path) {
            Ok(bytes) => {
                if let Err(e) = run_bytecode(&bytes) {
                    eprintln!("raytask: {e}");
                    process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("raytask: cannot read '{path}': {e}");
                process::exit(1);
            }
        }
        return;
    }

    eprintln!("raytask-stub: no embedded bytecode");
    eprintln!("usage: raytask-stub [program.rtbc]");
    eprintln!("hint:  build an app with: raytask build main.rt --target app");
    process::exit(2);
}
