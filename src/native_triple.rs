//! Native OS × architecture triples for AOT and the built-in linker.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arch {
    X86_64,
    Aarch64,
    Arm,
    I686,
}

impl Arch {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace('-', "_").as_str() {
            "x86_64" | "x64" | "amd64" | "x86-64" => Some(Self::X86_64),
            "aarch64" | "arm64" => Some(Self::Aarch64),
            "arm" | "armv7" | "armv7a" | "thumb" => Some(Self::Arm),
            "i686" | "i386" | "x86" | "ia32" => Some(Self::I686),
            "current" | "host" | "native" => Some(Self::host()),
            _ => None,
        }
    }

    pub fn host() -> Self {
        if cfg!(target_arch = "aarch64") {
            Self::Aarch64
        } else if cfg!(target_arch = "arm") {
            Self::Arm
        } else if cfg!(target_arch = "x86") {
            Self::I686
        } else {
            Self::X86_64
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::Arm => "arm",
            Self::I686 => "i686",
        }
    }

    pub fn elf_machine(self) -> u16 {
        match self {
            Self::X86_64 => 62,   // EM_X86_64
            Self::Aarch64 => 183, // EM_AARCH64
            Self::Arm => 40,      // EM_ARM
            Self::I686 => 3,      // EM_386
        }
    }

    pub fn coff_machine(self) -> u16 {
        match self {
            Self::X86_64 => 0x8664,
            Self::Aarch64 => 0xAA64,
            Self::Arm => 0x01C4, // IMAGE_FILE_MACHINE_ARMNT
            Self::I686 => 0x014C,
        }
    }

    pub fn pointer_size(self) -> usize {
        match self {
            Self::X86_64 | Self::Aarch64 => 8,
            Self::Arm | Self::I686 => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OsKind {
    Windows,
    Linux,
    Macos,
    Uefi,
    Freestanding,
}

impl OsKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "windows" | "win" | "win32" | "win64" => Some(Self::Windows),
            "linux" => Some(Self::Linux),
            "macos" | "mac" | "darwin" | "osx" => Some(Self::Macos),
            "uefi" | "efi" => Some(Self::Uefi),
            "freestanding" | "none" | "bare" | "none-eabi" | "raw" => Some(Self::Freestanding),
            "current" | "host" => Some(Self::host()),
            _ => None,
        }
    }

    pub fn host() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Uefi => "uefi",
            Self::Freestanding => "freestanding",
        }
    }
}

/// Fully-qualified native build / link target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeTriple {
    pub os: OsKind,
    pub arch: Arch,
}

impl NativeTriple {
    pub fn new(os: OsKind, arch: Arch) -> Self {
        Self { os, arch }
    }

    pub fn host() -> Self {
        Self {
            os: OsKind::host(),
            arch: Arch::host(),
        }
    }

    /// Parse `"linux-aarch64"`, `"windows-x64"`, `"macos"`, `"aarch64-linux-gnu"`, etc.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        if matches!(s.as_str(), "current" | "host" | "native") {
            return Some(Self::host());
        }
        // clang-style: aarch64-unknown-linux-gnu / x86_64-pc-windows-msvc
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() >= 2 {
            if let Some(arch) = Arch::parse(parts[0]) {
                let os = if parts.iter().any(|p| matches!(*p, "windows" | "win32" | "msvc")) {
                    Some(OsKind::Windows)
                } else if parts.iter().any(|p| matches!(*p, "linux" | "musl")) {
                    Some(OsKind::Linux)
                } else if parts.iter().any(|p| matches!(*p, "apple" | "darwin" | "macos")) {
                    Some(OsKind::Macos)
                } else if parts.iter().any(|p| matches!(*p, "uefi" | "efi")) {
                    Some(OsKind::Uefi)
                } else if parts.iter().any(|p| matches!(*p, "none" | "eabi" | "elf")) {
                    Some(OsKind::Freestanding)
                } else {
                    None
                };
                if let Some(os) = os {
                    return Some(Self { os, arch });
                }
            }
        }
        // os-arch: linux-aarch64, windows-x86_64
        if let Some((a, b)) = s.split_once('-') {
            if let (Some(os), Some(arch)) = (OsKind::parse(a), Arch::parse(b)) {
                return Some(Self { os, arch });
            }
            if let (Some(arch), Some(os)) = (Arch::parse(a), OsKind::parse(b)) {
                return Some(Self { os, arch });
            }
        }
        // bare os → host arch
        if let Some(os) = OsKind::parse(&s) {
            return Some(Self {
                os,
                arch: Arch::host(),
            });
        }
        // bare arch → host os
        if let Some(arch) = Arch::parse(&s) {
            return Some(Self {
                os: OsKind::host(),
                arch,
            });
        }
        None
    }

    pub fn name(self) -> String {
        format!("{}-{}", self.os.name(), self.arch.name())
    }

    /// clang/zig `-target` style triple.
    pub fn clang_target(self) -> &'static str {
        match (self.os, self.arch) {
            (OsKind::Windows, Arch::X86_64) => "x86_64-pc-windows-msvc",
            (OsKind::Windows, Arch::Aarch64) => "aarch64-pc-windows-msvc",
            (OsKind::Windows, Arch::I686) => "i686-pc-windows-msvc",
            (OsKind::Windows, Arch::Arm) => "armv7-pc-windows-msvc",
            (OsKind::Linux, Arch::X86_64) => "x86_64-unknown-linux-gnu",
            (OsKind::Linux, Arch::Aarch64) => "aarch64-unknown-linux-gnu",
            (OsKind::Linux, Arch::Arm) => "armv7-unknown-linux-gnueabihf",
            (OsKind::Linux, Arch::I686) => "i686-unknown-linux-gnu",
            (OsKind::Macos, Arch::X86_64) => "x86_64-apple-darwin",
            (OsKind::Macos, Arch::Aarch64) => "aarch64-apple-darwin",
            (OsKind::Macos, Arch::I686) => "i686-apple-darwin",
            (OsKind::Macos, Arch::Arm) => "armv7-apple-darwin",
            (OsKind::Uefi, Arch::X86_64) => "x86_64-unknown-uefi",
            (OsKind::Uefi, Arch::Aarch64) => "aarch64-unknown-uefi",
            (OsKind::Uefi, _) => "x86_64-unknown-uefi",
            (OsKind::Freestanding, Arch::X86_64) => "x86_64-unknown-none",
            (OsKind::Freestanding, Arch::Aarch64) => "aarch64-unknown-none",
            (OsKind::Freestanding, Arch::Arm) => "armv7-unknown-none-eabi",
            (OsKind::Freestanding, Arch::I686) => "i686-unknown-none",
        }
    }

    pub fn default_ext(self) -> &'static str {
        match self.os {
            OsKind::Windows | OsKind::Uefi => {
                if self.os == OsKind::Uefi {
                    "efi"
                } else {
                    "exe"
                }
            }
            OsKind::Macos => "macho",
            OsKind::Linux => "elf",
            OsKind::Freestanding => "bin",
        }
    }

    pub fn is_cross(self) -> bool {
        self != Self::host()
    }

    pub fn matches_host(self) -> bool {
        !self.is_cross()
    }
}

impl fmt::Display for NativeTriple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_common_triples() {
        assert_eq!(
            NativeTriple::parse("linux-aarch64").unwrap().arch,
            Arch::Aarch64
        );
        assert_eq!(
            NativeTriple::parse("windows-x64").unwrap().os,
            OsKind::Windows
        );
        assert_eq!(
            NativeTriple::parse("aarch64-unknown-linux-gnu")
                .unwrap()
                .arch,
            Arch::Aarch64
        );
        assert_eq!(NativeTriple::parse("host").unwrap(), NativeTriple::host());
    }
}
