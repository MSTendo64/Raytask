//! Simple preprocessor for `#if` / `#elif` / `#else` / `#endif` and `#pragma`.

use std::collections::HashSet;

struct IfState {
    parent_emitting: bool,
    branch_taken: bool,
}

/// Expand preprocessor directives before lexing.
pub fn preprocess(source: &str, defs: &HashSet<String>) -> String {
    let mut out = String::new();
    let mut stack: Vec<IfState> = Vec::new();
    let mut emitting = true;

    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let rest = rest.trim_start();
            if rest.starts_with("if ") || rest.starts_with("IF ") {
                let cond = rest[3..].trim();
                let active = emitting && eval_cond(cond, defs);
                stack.push(IfState {
                    parent_emitting: emitting,
                    branch_taken: active,
                });
                emitting = active;
                continue;
            }
            if rest.starts_with("elif ") || rest.starts_with("ELIF ") {
                let cond = rest[5..].trim();
                if let Some(state) = stack.last() {
                    if state.parent_emitting && !state.branch_taken {
                        let active = eval_cond(cond, defs);
                        if let Some(state) = stack.last_mut() {
                            state.branch_taken = active;
                        }
                        emitting = active;
                    } else {
                        emitting = false;
                    }
                }
                continue;
            }
            if rest.eq_ignore_ascii_case("else") {
                if let Some(state) = stack.last() {
                    if state.parent_emitting && !state.branch_taken {
                        emitting = true;
                        if let Some(state) = stack.last_mut() {
                            state.branch_taken = true;
                        }
                    } else {
                        emitting = false;
                    }
                }
                continue;
            }
            if rest.eq_ignore_ascii_case("endif") {
                if let Some(state) = stack.pop() {
                    emitting = state.parent_emitting;
                }
                continue;
            }
            if rest.starts_with("pragma ") {
                // Ignored for now (warning disable/restore)
                continue;
            }
            // Unknown directive — keep line if emitting
        }
        if emitting {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn eval_cond(cond: &str, defs: &HashSet<String>) -> bool {
    let c = cond.trim();
    if let Some(rest) = c.strip_prefix('!') {
        return !defs.contains(rest.trim());
    }
    // Support `DEFINED` style or bare names
    defs.contains(c)
}

pub fn default_defs(debug: bool) -> HashSet<String> {
    let mut d = HashSet::new();
    if debug {
        d.insert("DEBUG".into());
    } else {
        d.insert("RELEASE".into());
    }
    if cfg!(windows) {
        d.insert("WINDOWS".into());
    }
    if cfg!(target_os = "linux") {
        d.insert("LINUX".into());
    }
    if cfg!(target_os = "macos") {
        d.insert("MACOS".into());
        d.insert("OSX".into());
    }
    if cfg!(target_arch = "x86_64") {
        d.insert("X86_64".into());
    }
    if cfg!(target_arch = "aarch64") {
        d.insert("ARM64".into());
        d.insert("AARCH64".into());
    }
    d
}
