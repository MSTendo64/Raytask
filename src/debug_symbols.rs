//! RayTask debug symbols (`.rtdbg`) — sidecar for bytecode / native builds.

use crate::bytecode::{Chunk, LocalDebug, Module};
use crate::error::{CompileError, CompileResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const RTDBG_MAGIC: &str = "RTDBG";
pub const RTDBG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugSymbols {
    pub magic: String,
    pub version: u32,
    /// Entry source file used for this build.
    pub entry: String,
    /// Path of the companion artifact (.rtbc / .exe / …), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    pub main_chunk: usize,
    pub globals: Vec<String>,
    pub chunks: Vec<ChunkSymbols>,
    pub classes: Vec<ClassSymbols>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkSymbols {
    pub index: usize,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub arity: usize,
    pub local_count: usize,
    pub is_async: bool,
    pub code_len: usize,
    /// Parallel to bytecode: source line per code byte (0 = unknown).
    pub lines: Vec<usize>,
    pub locals: Vec<LocalSymbols>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSymbols {
    pub name: String,
    pub slot: u8,
    pub start_ip: usize,
    /// Exclusive; `null` in JSON means live until end of chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ip: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassSymbols {
    pub name: String,
    pub fields: Vec<String>,
    pub methods: Vec<(String, usize)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constructor: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructor: Option<usize>,
}

impl DebugSymbols {
    pub fn from_module(module: &Module, entry: &Path, artifact: Option<&Path>) -> Self {
        let chunks = module
            .chunks
            .iter()
            .enumerate()
            .map(|(index, c)| ChunkSymbols {
                index,
                name: c.name.clone(),
                source: c.source.clone(),
                arity: c.arity,
                local_count: c.local_count,
                is_async: c.is_async,
                code_len: c.code.len(),
                lines: c.lines.clone(),
                locals: c
                    .local_debug
                    .iter()
                    .map(|ld| LocalSymbols {
                        name: ld.name.clone(),
                        slot: ld.slot,
                        start_ip: ld.start_ip,
                        end_ip: if ld.end_ip == usize::MAX || ld.end_ip >= c.code.len() {
                            None
                        } else {
                            Some(ld.end_ip)
                        },
                    })
                    .collect(),
            })
            .collect();

        let classes = module
            .classes
            .iter()
            .map(|c| ClassSymbols {
                name: c.name.clone(),
                fields: c.fields.clone(),
                methods: c.methods.clone(),
                constructor: c.constructor,
                base: c.base,
                destructor: c.destructor,
            })
            .collect();

        Self {
            magic: RTDBG_MAGIC.into(),
            version: RTDBG_VERSION,
            entry: entry.display().to_string(),
            artifact: artifact.map(|p| p.display().to_string()),
            main_chunk: module.main_chunk,
            globals: module.globals.clone(),
            chunks,
            classes,
        }
    }

    pub fn to_json_pretty(&self) -> CompileResult<String> {
        serde_json::to_string_pretty(self).map_err(|e| CompileError::Io {
            message: format!("serialize debug symbols: {e}"),
        })
    }

    pub fn write_file(&self, path: &Path) -> CompileResult<()> {
        let text = self.to_json_pretty()?;
        std::fs::write(path, text).map_err(|e| CompileError::Io {
            message: format!("{}: {e}", path.display()),
        })
    }

    pub fn read_file(path: &Path) -> CompileResult<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| CompileError::Io {
            message: format!("{}: {e}", path.display()),
        })?;
        let sym: Self = serde_json::from_str(&text).map_err(|e| CompileError::Io {
            message: format!("invalid .rtdbg '{}': {e}", path.display()),
        })?;
        if sym.magic != RTDBG_MAGIC {
            return Err(CompileError::Io {
                message: format!(
                    "bad .rtdbg magic in '{}' (expected {RTDBG_MAGIC})",
                    path.display()
                ),
            });
        }
        if sym.version > RTDBG_VERSION {
            return Err(CompileError::Io {
                message: format!(
                    "unsupported .rtdbg version {} in '{}' (max {RTDBG_VERSION})",
                    sym.version,
                    path.display()
                ),
            });
        }
        Ok(sym)
    }

    /// Overlay symbols onto a loaded module (fills empty local_debug / source / lines).
    pub fn apply_to_module(&self, module: &mut Module) {
        for cs in &self.chunks {
            let Some(chunk) = module.chunks.get_mut(cs.index) else {
                continue;
            };
            if chunk.source.is_none() {
                chunk.source = cs.source.clone();
            }
            if chunk.local_debug.is_empty() && !cs.locals.is_empty() {
                let end_fallback = chunk.code.len().max(cs.code_len);
                chunk.local_debug = cs
                    .locals
                    .iter()
                    .map(|l| LocalDebug {
                        name: l.name.clone(),
                        slot: l.slot,
                        start_ip: l.start_ip,
                        end_ip: l.end_ip.unwrap_or(end_fallback),
                    })
                    .collect();
            }
            if (chunk.lines.is_empty() || chunk.lines.iter().all(|&l| l == 0))
                && !cs.lines.is_empty()
                && cs.lines.len() == chunk.code.len()
            {
                chunk.lines = cs.lines.clone();
            } else if chunk.lines.len() != chunk.code.len() && cs.lines.len() == chunk.code.len() {
                chunk.lines = cs.lines.clone();
            }
            if chunk.name.is_empty() || chunk.name.starts_with('<') {
                // keep synthetic names; only fill if blank
            }
            if chunk.local_count < cs.local_count {
                chunk.local_count = cs.local_count;
            }
        }
        if module.globals.is_empty() && !self.globals.is_empty() {
            module.globals = self.globals.clone();
        }
    }
}

/// Default sidecar path for an artifact: `foo.rtbc` → `foo.rtdbg`, `foo.rt` → `foo.rtdbg`.
pub fn sidecar_path(artifact_or_source: &Path) -> PathBuf {
    artifact_or_source.with_extension("rtdbg")
}

/// Find `.rtdbg` next to a program path (`.rt`, `.rtbc`, or exe).
pub fn find_sidecar(program: &Path) -> Option<PathBuf> {
    let candidates = [
        sidecar_path(program),
        program.with_extension("rtdbg"),
        {
            let mut p = program.to_path_buf();
            p.set_extension("");
            let stem = p.file_name().map(|s| s.to_os_string()).unwrap_or_default();
            program
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!("{}.rtdbg", stem.to_string_lossy()))
        },
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// Strip heavy debug tables from a module (release builds). Line table is kept.
pub fn strip_module_debug(module: &mut Module) {
    for chunk in &mut module.chunks {
        chunk.local_debug.clear();
        chunk.source = None;
    }
}

/// Ensure every chunk has a source path for symbols / DAP.
pub fn stamp_source(module: &mut Module, source: &Path) {
    let s = source.display().to_string();
    for chunk in &mut module.chunks {
        if chunk.source.is_none() {
            chunk.source = Some(s.clone());
        }
    }
}

/// Helper used when only a Chunk list is available during compile.
pub fn chunk_has_locals(chunk: &Chunk) -> bool {
    !chunk.local_debug.is_empty()
}
