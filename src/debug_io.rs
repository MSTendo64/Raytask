//! Redirect program stdout/stderr during DAP sessions.

use std::cell::RefCell;
use std::io::{self, Write};
use std::sync::mpsc::Sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugOutputKind {
    Stdout,
    Stderr,
}

thread_local! {
    static SINK: RefCell<Option<Sender<(DebugOutputKind, String)>>> = const { RefCell::new(None) };
}

/// Install a sink; program prints are sent here instead of the process stdout.
pub fn install(tx: Sender<(DebugOutputKind, String)>) {
    SINK.with(|s| *s.borrow_mut() = Some(tx));
}

pub fn uninstall() {
    SINK.with(|s| *s.borrow_mut() = None);
}

pub fn is_capturing() -> bool {
    SINK.with(|s| s.borrow().is_some())
}

pub fn write_stdout(msg: &str) {
    emit(DebugOutputKind::Stdout, msg, true);
}

pub fn write_stdout_raw(msg: &str) {
    emit(DebugOutputKind::Stdout, msg, false);
}

pub fn write_stderr(msg: &str) {
    emit(DebugOutputKind::Stderr, msg, true);
}

fn emit(kind: DebugOutputKind, msg: &str, ensure_nl: bool) {
    let sent = SINK.with(|s| {
        if let Some(tx) = s.borrow().as_ref() {
            let mut text = msg.to_string();
            if ensure_nl && !text.ends_with('\n') {
                text.push('\n');
            }
            tx.send((kind, text)).is_ok()
        } else {
            false
        }
    });
    if !sent {
        match kind {
            DebugOutputKind::Stdout => {
                if ensure_nl {
                    println!("{}", msg.trim_end_matches('\n'));
                } else {
                    print!("{}", msg);
                    let _ = io::stdout().flush();
                }
            }
            DebugOutputKind::Stderr => {
                if ensure_nl {
                    eprintln!("{}", msg.trim_end_matches('\n'));
                } else {
                    eprint!("{}", msg);
                }
            }
        }
    }
}
