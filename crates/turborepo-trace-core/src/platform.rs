//! The target platform: OS, CPU architecture and C library, shared by every
//! language.
//!
//! One [`Platform`] per tracer comes from [`crate::TraceOptions::platform()`]:
//! an explicit `platform` option, else `--target` parsed with
//! [`Platform::parse`], else the host. Languages map it to their ecosystem's
//! names:
//!
//! | Ecosystem | Methods |
//! |---|---|
//! | Node (`process.platform` / `process.arch`, npm package names) | [`Platform::node_os`], [`Platform::node_arch`], [`Platform::os_arch`], [`Platform::matches_name`] |
//! | Python (extension suffixes, wheel tags, `sys.platform`) | [`Platform::python_ext_suffixes`], [`Platform::python_multiarch`], [`Platform::python_wheel_tag_matches`], [`Platform::python_sys_platform`] |
//! | Go (`GOOS` / `GOARCH`, build tags, `_GOOS_GOARCH` file names) | [`Platform::go_os`], [`Platform::go_arch`], [`Platform::go_tag_matches`], [`Platform::go_file_matches`] |
//! | Rust (target triple, `cfg(target_*)`) | [`Platform::rust_triple`], [`Platform::rust_target_os`], [`Platform::rust_target_arch`], [`Platform::rust_target_env`], [`Platform::rust_cfg`] |
//!
//! Shared libraries for the platform: [`Platform::is_sharedlib`] /
//! [`is_sharedlib_for`].

use memchr::memmem;

/// Which C library a Linux target uses. Packages ship separate binaries for
/// glibc and musl (`@img/sharp-linuxmusl-x64`, `manylinux` vs `musllinux`
/// wheels, `*-linux-musl` triples).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Libc {
    /// GNU libc (most distributions).
    Glibc,
    /// musl (Alpine).
    Musl,
    /// Not applicable (macOS, Windows, ...).
    None,
}

impl Libc {
    /// node-pre-gyp's `{libc}` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Libc::Glibc => "glibc",
            Libc::Musl => "musl",
            Libc::None => "unknown",
        }
    }
}

/// Operating system.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Os {
    /// Linux.
    Linux,
    /// macOS.
    Darwin,
    /// Windows.
    Windows,
    /// FreeBSD.
    FreeBsd,
    /// OpenBSD.
    OpenBsd,
    /// NetBSD.
    NetBsd,
    /// Solaris / SunOS.
    SunOs,
    /// Android.
    Android,
    /// AIX.
    Aix,
    /// WebAssembly (WASI).
    Wasm,
    /// A host OS without a mapping (`std::env::consts::OS`), named as-is
    /// everywhere.
    Other(&'static str),
}

/// CPU architecture.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Arch {
    /// x86-64 / amd64.
    X86_64,
    /// AArch64 / arm64.
    Aarch64,
    /// 32-bit x86.
    X86,
    /// 32-bit ARM.
    Arm,
    /// 64-bit PowerPC (little endian in Go, Python and Rust names).
    Ppc64,
    /// IBM Z.
    S390x,
    /// 64-bit RISC-V.
    Riscv64,
    /// 64-bit LoongArch.
    Loongarch64,
    /// 64-bit little-endian MIPS.
    Mips64el,
    /// A host architecture without a mapping (`std::env::consts::ARCH`), named
    /// as-is.
    Other(&'static str),
}

/// Target OS, architecture and libc.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Platform {
    /// Operating system.
    pub os: Os,
    /// CPU architecture.
    pub arch: Arch,
    /// C library ([`Libc::None`] off Linux).
    pub libc: Libc,
}

/// OS names that appear in platform-specific package and file names, and in
/// `--target`.
const OS_TOKENS: &[(&str, Os)] = &[
    ("darwin", Os::Darwin),
    ("macos", Os::Darwin),
    ("linux", Os::Linux),
    ("linuxmusl", Os::Linux),
    ("win32", Os::Windows),
    ("windows", Os::Windows),
    ("win", Os::Windows),
    ("freebsd", Os::FreeBsd),
    ("openbsd", Os::OpenBsd),
    ("netbsd", Os::NetBsd),
    ("sunos", Os::SunOs),
    ("android", Os::Android),
    ("aix", Os::Aix),
    ("wasm32", Os::Wasm),
    ("wasi", Os::Wasm),
];

/// Architecture names in package and file names, and in `--target`.
const ARCH_TOKENS: &[(&str, Arch)] = &[
    ("x64", Arch::X86_64),
    ("amd64", Arch::X86_64),
    ("x86_64", Arch::X86_64),
    ("64", Arch::X86_64),
    ("arm64", Arch::Aarch64),
    ("aarch64", Arch::Aarch64),
    ("ia32", Arch::X86),
    ("x86", Arch::X86),
    ("32", Arch::X86),
    ("arm", Arch::Arm),
    ("armv7", Arch::Arm),
    ("armv6", Arch::Arm),
    ("ppc64", Arch::Ppc64),
    ("ppc64le", Arch::Ppc64),
    ("s390x", Arch::S390x),
    ("riscv64", Arch::Riscv64),
    ("loong64", Arch::Loongarch64),
    ("mips64el", Arch::Mips64el),
];

/// Extra spellings accepted by [`Platform::parse`] only (Go, Rust and Python
/// names). They are not used by [`Platform::matches_name`], whose token set
/// follows npm package naming.
const PARSE_OS_ALIASES: &[(&str, Os)] = &[("solaris", Os::SunOs), ("wasip1", Os::Wasm)];
const PARSE_ARCH_ALIASES: &[(&str, Arch)] = &[
    ("386", Arch::X86),
    ("i386", Arch::X86),
    ("i586", Arch::X86),
    ("i686", Arch::X86),
    ("armv7l", Arch::Arm),
    ("powerpc64le", Arch::Ppc64),
    ("powerpc64", Arch::Ppc64),
    ("riscv64gc", Arch::Riscv64),
    ("loongarch64", Arch::Loongarch64),
    ("mips64le", Arch::Mips64el),
];

fn lookup<T: Copy>(table: &[(&str, T)], s: &str) -> Option<T> {
    table.iter().find(|(k, _)| *k == s).map(|(_, v)| *v)
}

fn os_token(s: &str) -> Option<Os> {
    lookup(OS_TOKENS, s)
}

fn arch_token(s: &str) -> Option<Arch> {
    lookup(ARCH_TOKENS, s)
}

fn parse_os(s: &str) -> Option<Os> {
    os_token(s).or_else(|| lookup(PARSE_OS_ALIASES, s))
}

fn parse_arch(s: &str) -> Option<Arch> {
    arch_token(s).or_else(|| lookup(PARSE_ARCH_ALIASES, s))
}

impl Os {
    /// Node's `process.platform`: `linux`, `darwin`, `win32`, `freebsd`,
    /// `sunos`, ...
    pub const fn node(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Darwin => "darwin",
            Os::Windows => "win32",
            Os::FreeBsd => "freebsd",
            Os::OpenBsd => "openbsd",
            Os::NetBsd => "netbsd",
            Os::SunOs => "sunos",
            Os::Android => "android",
            Os::Aix => "aix",
            Os::Wasm => "wasm32",
            Os::Other(s) => s,
        }
    }

    /// Go's `GOOS`.
    pub const fn go(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Darwin => "darwin",
            Os::Windows => "windows",
            Os::FreeBsd => "freebsd",
            Os::OpenBsd => "openbsd",
            Os::NetBsd => "netbsd",
            Os::SunOs => "solaris",
            Os::Android => "android",
            Os::Aix => "aix",
            Os::Wasm => "wasip1",
            Os::Other(s) => s,
        }
    }

    /// Rust's `cfg(target_os)`.
    pub const fn rust(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Darwin => "macos",
            Os::Windows => "windows",
            Os::FreeBsd => "freebsd",
            Os::OpenBsd => "openbsd",
            Os::NetBsd => "netbsd",
            Os::SunOs => "solaris",
            Os::Android => "android",
            Os::Aix => "aix",
            Os::Wasm => "wasi",
            Os::Other(s) => s,
        }
    }

    /// Whether the OS is a Unix (Go's `unix` build tag, Rust's `cfg(unix)`).
    pub const fn is_unix(self) -> bool {
        !matches!(self, Os::Windows | Os::Wasm)
    }
}

impl Arch {
    /// Node's `process.arch`: `x64`, `arm64`, `ia32`, `arm`, ...
    pub const fn node(self) -> &'static str {
        match self {
            Arch::X86_64 => "x64",
            Arch::Aarch64 => "arm64",
            Arch::X86 => "ia32",
            Arch::Arm => "arm",
            Arch::Ppc64 => "ppc64",
            Arch::S390x => "s390x",
            Arch::Riscv64 => "riscv64",
            Arch::Loongarch64 => "loong64",
            Arch::Mips64el => "mips64el",
            Arch::Other(s) => s,
        }
    }

    /// Go's `GOARCH`.
    pub const fn go(self) -> &'static str {
        match self {
            Arch::X86_64 => "amd64",
            Arch::Aarch64 => "arm64",
            Arch::X86 => "386",
            Arch::Arm => "arm",
            Arch::Ppc64 => "ppc64le",
            Arch::S390x => "s390x",
            Arch::Riscv64 => "riscv64",
            Arch::Loongarch64 => "loong64",
            Arch::Mips64el => "mips64le",
            Arch::Other(s) => s,
        }
    }

    /// Rust's `cfg(target_arch)`.
    pub const fn rust(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
            Arch::X86 => "x86",
            Arch::Arm => "arm",
            Arch::Ppc64 => "powerpc64",
            Arch::S390x => "s390x",
            Arch::Riscv64 => "riscv64",
            Arch::Loongarch64 => "loongarch64",
            Arch::Mips64el => "mips64",
            Arch::Other(s) => s,
        }
    }

    /// Python's `platform.machine()` on Linux (`x86_64`, `aarch64`, `i686`,
    /// `armv7l`, ...), as used in `manylinux_*_<machine>` wheel tags.
    pub const fn python_linux_machine(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
            Arch::X86 => "i686",
            Arch::Arm => "armv7l",
            Arch::Ppc64 => "ppc64le",
            Arch::S390x => "s390x",
            Arch::Riscv64 => "riscv64",
            Arch::Loongarch64 => "loongarch64",
            Arch::Mips64el => "mips64",
            Arch::Other(s) => s,
        }
    }

    /// Pointer width in bits.
    pub const fn pointer_width(self) -> u32 {
        match self {
            Arch::X86 | Arch::Arm => 32,
            _ => 64,
        }
    }
}

impl Default for Platform {
    fn default() -> Self {
        Self::host()
    }
}

impl Platform {
    /// The host platform.
    pub fn host() -> Self {
        let os = os_token(std::env::consts::OS).unwrap_or(Os::Other(std::env::consts::OS));
        let arch =
            arch_token(std::env::consts::ARCH).unwrap_or(Arch::Other(std::env::consts::ARCH));
        let libc = if os != Os::Linux {
            Libc::None
        } else if cfg!(target_env = "musl") {
            Libc::Musl
        } else {
            Libc::Glibc
        };
        Platform { os, arch, libc }
    }

    /// Parses a `--target` value. Accepted forms:
    ///
    /// - `<os>/<arch>[/<libc>]`, also with `-` separators, in Node or Go names:
    ///   `linux/x64`, `linux/amd64`, `linux-x64-musl`, `linuxmusl/arm64`,
    ///   `darwin/arm64`, `macos/aarch64`, `windows/386`;
    /// - a Rust target triple: `x86_64-unknown-linux-gnu`,
    ///   `aarch64-unknown-linux-musl`, `aarch64-apple-darwin`,
    ///   `x86_64-pc-windows-msvc`.
    ///
    /// Returns `None` if the OS or architecture is not recognized.
    pub fn parse(target: &str) -> Option<Self> {
        Self::parse_os_first(target).or_else(|| Self::parse_triple(target))
    }

    fn parse_os_first(target: &str) -> Option<Self> {
        let mut it = target.split(['/', '-']).filter(|s| !s.is_empty());
        let os_raw = it.next()?.to_ascii_lowercase();
        let os = parse_os(&os_raw)?;
        let arch = parse_arch(&it.next()?.to_ascii_lowercase())?;
        let libc = match (os, it.next().map(|s| s.to_ascii_lowercase())) {
            (Os::Linux, Some(l)) if l == "musl" => Libc::Musl,
            (Os::Linux, _) if os_raw == "linuxmusl" => Libc::Musl,
            (Os::Linux, _) => Libc::Glibc,
            _ => Libc::None,
        };
        Some(Platform { os, arch, libc })
    }

    /// `<arch>-<vendor>-<os>[-<env>]` or `<arch>-<os>[-<env>]`.
    fn parse_triple(target: &str) -> Option<Self> {
        let lower = target.to_ascii_lowercase();
        let mut it = lower.split('-').filter(|s| !s.is_empty());
        let arch = parse_arch(it.next()?)?;
        let rest: Vec<&str> = it.collect();
        let (pos, os) = rest
            .iter()
            .enumerate()
            .find_map(|(i, t)| parse_os(t).map(|os| (i, os)))?;
        let env = rest.get(pos + 1).copied().unwrap_or("");
        let libc = match os {
            Os::Linux if env.starts_with("musl") => Libc::Musl,
            Os::Linux => Libc::Glibc,
            _ => Libc::None,
        };
        Some(Platform { os, arch, libc })
    }

    // ---- Node ----

    /// `process.platform`.
    pub const fn node_os(&self) -> &'static str {
        self.os.node()
    }

    /// `process.arch`.
    pub const fn node_arch(&self) -> &'static str {
        self.arch.node()
    }

    /// `<os>-<arch>` in Node names, as in `prebuilds/linux-x64`.
    pub fn os_arch(&self) -> String {
        format!("{}-{}", self.node_os(), self.node_arch())
    }

    /// Whether a platform-specific name (package such as
    /// `@img/sharp-linuxmusl-x64`, `@esbuild/darwin-arm64`,
    /// `@next/swc-linux-x64-gnu`, or a file name) is compatible:
    /// it must not mention another OS, another architecture, or the other libc.
    /// Names that mention no platform at all are compatible. Tokens are
    /// split at `- _ / . @ +` and follow npm naming (Node and common
    /// aliases).
    pub fn matches_name(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        let tokens: Vec<&str> = lower
            .split(['-', '_', '/', '.', '@', '+'])
            .filter(|t| !t.is_empty())
            .collect();
        let mut saw_os = false;
        for t in &tokens {
            if let Some(os) = os_token(t) {
                saw_os = true;
                if os != self.os {
                    return false;
                }
                if *t == "linuxmusl" && self.libc != Libc::Musl {
                    return false;
                }
            }
        }
        for t in &tokens {
            // Bare `32`/`64` are architectures only next to an OS (`esbuild-linux-64`).
            if (*t == "32" || *t == "64") && !saw_os {
                continue;
            }
            if *t == "universal" && self.os == Os::Darwin {
                continue;
            }
            if let Some(arch) = arch_token(t)
                && arch != self.arch
            {
                return false;
            }
        }
        if self.os == Os::Linux {
            let musl = tokens.iter().any(|t| *t == "musl" || *t == "linuxmusl");
            let gnu = tokens.iter().any(|t| *t == "gnu" || *t == "glibc");
            match self.libc {
                Libc::Glibc if musl => return false,
                Libc::Musl if gnu => return false,
                _ => {}
            }
        }
        true
    }

    // ---- Shared libraries ----

    /// Whether a file name is a shared library for this platform (`libfoo.so`,
    /// `libfoo.so.1.2`, on macOS also `libfoo.dylib`, on Windows only
    /// `foo.dll`), as in nft's `sharedlibEmit`. Node native addons
    /// (`*.node`, including Prisma's `*.so.node`) are not shared libraries
    /// here.
    pub fn is_sharedlib(&self, name: &[u8]) -> bool {
        is_sharedlib_for(self.os, name)
    }

    // ---- Python ----

    /// `sys.platform`: `linux`, `darwin`, `win32`, else the Rust OS name.
    pub const fn python_sys_platform(&self) -> &'static str {
        match self.os {
            Os::Linux | Os::Android => "linux",
            Os::Darwin => "darwin",
            Os::Windows => "win32",
            other => other.rust(),
        }
    }

    /// The platform part of CPython's `EXT_SUFFIX` / `SOABI`:
    /// `x86_64-linux-gnu`, `aarch64-linux-musl`, `arm-linux-gnueabihf`,
    /// `i386-linux-gnu`, `darwin`, `win_amd64`.
    pub fn python_multiarch(&self) -> String {
        match self.os {
            Os::Darwin => "darwin".into(),
            Os::Windows => match self.arch {
                Arch::X86 => "win32".into(),
                Arch::Aarch64 => "win_arm64".into(),
                _ => "win_amd64".into(),
            },
            Os::Linux => {
                let env = if self.libc == Libc::Musl {
                    "musl"
                } else {
                    "gnu"
                };
                match self.arch {
                    Arch::Arm => format!("arm-linux-{env}eabihf"),
                    Arch::X86 => format!("i386-linux-{env}"),
                    Arch::Ppc64 => format!("powerpc64le-linux-{env}"),
                    Arch::Mips64el => format!("mips64el-linux-{env}abi64"),
                    a => format!("{}-linux-{env}", a.python_linux_machine()),
                }
            }
            other => format!("{}-{}", self.arch.python_linux_machine(), other.rust()),
        }
    }

    /// CPython's `importlib.machinery.EXTENSION_SUFFIXES` for an interpreter
    /// tag such as `cpython-312`, most specific first:
    /// `.cpython-312-x86_64-linux-gnu.so`, `.abi3.so`, `.so` (on Windows
    /// `.cp312-win_amd64.pyd`, `.pyd`).
    pub fn python_ext_suffixes(&self, interpreter: &str) -> Vec<String> {
        if self.os == Os::Windows {
            let short = interpreter
                .strip_prefix("cpython-")
                .map(|v| format!("cp{v}"))
                .unwrap_or_else(|| interpreter.to_owned());
            return vec![
                format!(".{short}-{}.pyd", self.python_multiarch()),
                ".pyd".into(),
            ];
        }
        vec![
            format!(".{interpreter}-{}.so", self.python_multiarch()),
            ".abi3.so".into(),
            ".so".into(),
        ]
    }

    /// Whether a wheel platform tag is installable here: `any`,
    /// `manylinux_2_17_x86_64`, `manylinux2014_aarch64`,
    /// `musllinux_1_1_x86_64`, `linux_x86_64`, `macosx_11_0_arm64`,
    /// `macosx_10_9_universal2`, `win_amd64`, ... Compressed tag sets
    /// (`manylinux_2_17_x86_64.manylinux2014_x86_64`) match if any member does.
    /// Glibc and musl versions are not checked.
    pub fn python_wheel_tag_matches(&self, tag: &str) -> bool {
        tag.split('.').any(|t| self.wheel_tag_matches_one(t))
    }

    fn wheel_tag_matches_one(&self, tag: &str) -> bool {
        if tag == "any" {
            return true;
        }
        match self.os {
            Os::Linux => {
                let machine = self.arch.python_linux_machine();
                let libc_ok = if tag.starts_with("musllinux") {
                    self.libc == Libc::Musl
                } else if tag.starts_with("manylinux") {
                    self.libc != Libc::Musl
                } else {
                    tag.starts_with("linux_")
                };
                libc_ok
                    && tag
                        .strip_suffix(machine)
                        .is_some_and(|rest| rest.ends_with('_'))
            }
            Os::Darwin => {
                let Some(rest) = tag.strip_prefix("macosx_") else {
                    return false;
                };
                let arch = rest.splitn(3, '_').nth(2).unwrap_or("");
                match self.arch {
                    Arch::Aarch64 => matches!(arch, "arm64" | "universal2"),
                    Arch::X86_64 => matches!(
                        arch,
                        "x86_64" | "intel" | "universal2" | "universal" | "fat64" | "fat3"
                    ),
                    _ => false,
                }
            }
            Os::Windows => match self.arch {
                Arch::X86_64 => tag == "win_amd64",
                Arch::X86 => tag == "win32",
                Arch::Aarch64 => tag == "win_arm64",
                _ => false,
            },
            _ => false,
        }
    }

    // ---- Go ----

    /// `GOOS`.
    pub const fn go_os(&self) -> &'static str {
        self.os.go()
    }

    /// `GOARCH`.
    pub const fn go_arch(&self) -> &'static str {
        self.arch.go()
    }

    /// Evaluates one build-constraint tag for this platform: `Some(true)` for
    /// the target's `GOOS` / `GOARCH`, `unix` on Unix, `linux` on Android,
    /// `solaris` on illumos; `Some(false)` for any other known `GOOS` /
    /// `GOARCH`; `None` for tags that are not about the platform (`cgo`,
    /// `go1.21`, custom tags), which the caller decides.
    pub fn go_tag_matches(&self, tag: &str) -> Option<bool> {
        if tag == self.go_os() || tag == self.go_arch() {
            return Some(true);
        }
        if tag == "unix" {
            return Some(self.os.is_unix());
        }
        if tag == "linux" && self.os == Os::Android {
            return Some(true);
        }
        (GO_KNOWN_OS.contains(&tag) || GO_KNOWN_ARCH.contains(&tag)).then_some(false)
    }

    /// Go's file-name constraint (`go/build` `goodOSArchFile`): `x_linux.go`,
    /// `x_amd64.go`, `x_linux_amd64.go` and the same before `_test` only build
    /// for that target. `name` is the file name with its extension.
    pub fn go_file_matches(&self, name: &str) -> bool {
        let stem = name.split_once('.').map_or(name, |(s, _)| s);
        let stem = stem.strip_suffix("_test").unwrap_or(stem);
        let parts: Vec<&str> = stem.split('_').collect();
        // The first element is the base name, never a constraint.
        let n = parts.len();
        let known_os = |s: &str| GO_KNOWN_OS.contains(&s);
        let known_arch = |s: &str| GO_KNOWN_ARCH.contains(&s);
        let os_ok = |s: &str| self.go_tag_matches(s) == Some(true);
        if n >= 3 && known_os(parts[n - 2]) && known_arch(parts[n - 1]) {
            return os_ok(parts[n - 2]) && parts[n - 1] == self.go_arch();
        }
        if n >= 2 {
            let last = parts[n - 1];
            if known_os(last) {
                return os_ok(last);
            }
            if known_arch(last) {
                return last == self.go_arch();
            }
        }
        true
    }

    // ---- Rust ----

    /// `cfg(target_os)`.
    pub const fn rust_target_os(&self) -> &'static str {
        self.os.rust()
    }

    /// `cfg(target_arch)`.
    pub const fn rust_target_arch(&self) -> &'static str {
        self.arch.rust()
    }

    /// `cfg(target_env)`: `gnu`, `musl`, `msvc` or empty.
    pub const fn rust_target_env(&self) -> &'static str {
        match (self.os, self.libc) {
            (Os::Linux, Libc::Musl) => "musl",
            (Os::Linux, _) => "gnu",
            (Os::Windows, _) => "msvc",
            _ => "",
        }
    }

    /// `cfg(target_family)`: `unix`, `windows` or `wasm`.
    pub const fn rust_target_family(&self) -> &'static str {
        match self.os {
            Os::Windows => "windows",
            Os::Wasm => "wasm",
            _ => "unix",
        }
    }

    /// The usual Rust target triple: `x86_64-unknown-linux-gnu`,
    /// `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`,
    /// `armv7-unknown-linux-gnueabihf`, ...
    pub fn rust_triple(&self) -> String {
        let arch = match self.arch {
            Arch::X86 => "i686",
            Arch::Arm => "armv7",
            Arch::Ppc64 => "powerpc64le",
            Arch::Riscv64 => "riscv64gc",
            Arch::Mips64el => "mips64el",
            a => a.rust(),
        };
        match self.os {
            Os::Linux => {
                let env = self.rust_target_env();
                let suffix = match self.arch {
                    Arch::Arm => "eabihf",
                    Arch::Mips64el => "abi64",
                    _ => "",
                };
                format!("{arch}-unknown-linux-{env}{suffix}")
            }
            Os::Darwin => format!("{arch}-apple-darwin"),
            Os::Windows => format!("{arch}-pc-windows-msvc"),
            Os::Android => format!("{arch}-linux-android"),
            Os::Wasm => "wasm32-wasip1".into(),
            Os::SunOs => format!("{arch}-pc-solaris"),
            Os::Aix => format!("{arch}-ibm-aix"),
            other => format!("{arch}-unknown-{}", other.rust()),
        }
    }

    /// Evaluates a `cfg` predicate leaf for this platform: `unix`, `windows`,
    /// and `target_os`, `target_arch`, `target_env`, `target_family`,
    /// `target_pointer_width` with a value. `None` for other keys
    /// (`feature`, `test`, `debug_assertions`, ...), which the caller
    /// decides.
    pub fn rust_cfg(&self, key: &str, value: Option<&str>) -> Option<bool> {
        let width = self.arch.pointer_width().to_string();
        let actual: &str = match (key, value) {
            ("unix", None) => return Some(self.rust_target_family() == "unix"),
            ("windows", None) => return Some(self.os == Os::Windows),
            ("target_os", Some(_)) => self.rust_target_os(),
            ("target_arch", Some(_)) => self.rust_target_arch(),
            ("target_env", Some(_)) => self.rust_target_env(),
            ("target_family", Some(_)) => self.rust_target_family(),
            ("target_pointer_width", Some(_)) => &width,
            _ => return None,
        };
        Some(value == Some(actual))
    }
}

/// Every `GOOS` known to `go/build` (for file-name and build-tag constraints).
pub const GO_KNOWN_OS: &[&str] = &[
    "aix",
    "android",
    "darwin",
    "dragonfly",
    "freebsd",
    "hurd",
    "illumos",
    "ios",
    "js",
    "linux",
    "nacl",
    "netbsd",
    "openbsd",
    "plan9",
    "solaris",
    "wasip1",
    "windows",
    "zos",
];

/// Every `GOARCH` known to `go/build`.
pub const GO_KNOWN_ARCH: &[&str] = &[
    "386",
    "amd64",
    "amd64p32",
    "arm",
    "armbe",
    "arm64",
    "arm64be",
    "loong64",
    "mips",
    "mipsle",
    "mips64",
    "mips64le",
    "mips64p32",
    "mips64p32le",
    "ppc",
    "ppc64",
    "ppc64le",
    "riscv",
    "riscv64",
    "s390",
    "s390x",
    "sparc",
    "sparc64",
    "wasm",
];

/// [`Platform::is_sharedlib`] for an OS.
pub fn is_sharedlib_for(os: Os, name: &[u8]) -> bool {
    if name.ends_with(b".node") {
        return false;
    }
    if os == Os::Darwin && name.ends_with(b".dylib") {
        return true;
    }
    if os == Os::Windows {
        return name.ends_with(b".dll");
    }
    // `*.so` or `*.so.*`.
    memmem::find_iter(name, b".so").any(|i| i > 0 && (i + 3 == name.len() || name[i + 3] == b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Platform {
        Platform::parse(s).unwrap()
    }

    #[test]
    fn parses_targets() {
        assert_eq!(p("linux/amd64").node_arch(), "x64");
        assert_eq!(p("linux/x64").libc, Libc::Glibc);
        assert_eq!(p("linux-x64-musl").libc, Libc::Musl);
        assert_eq!(p("linuxmusl/arm64").libc, Libc::Musl);
        assert_eq!(p("darwin/arm64").node_os(), "darwin");
        assert_eq!(p("macos/aarch64").node_arch(), "arm64");
        assert_eq!(p("darwin/arm64").libc, Libc::None);
        assert!(Platform::parse("plan9/x64").is_none());
        assert!(Platform::parse("linux").is_none());
        // Go and Rust spellings.
        assert_eq!(p("windows/386").arch, Arch::X86);
        assert_eq!(p("linux/mips64le").arch, Arch::Mips64el);
        let t = p("x86_64-unknown-linux-musl");
        assert_eq!(
            (t.os, t.arch, t.libc),
            (Os::Linux, Arch::X86_64, Libc::Musl)
        );
        let t = p("aarch64-apple-darwin");
        assert_eq!(
            (t.os, t.arch, t.libc),
            (Os::Darwin, Arch::Aarch64, Libc::None)
        );
        assert_eq!(p("armv7-unknown-linux-gnueabihf").arch, Arch::Arm);
        assert_eq!(p("x86_64-pc-windows-msvc").os, Os::Windows);
        assert!(Platform::parse("x86_64-unknown-plan9").is_none());
    }

    #[test]
    fn host_names_match_std() {
        let h = Platform::host();
        if cfg!(target_os = "linux") {
            assert_eq!(h.os, Os::Linux);
            assert_eq!(h.rust_target_os(), "linux");
        }
        if cfg!(target_os = "macos") {
            assert_eq!(h.node_os(), "darwin");
            assert_eq!(h.rust_target_os(), "macos");
        }
        if cfg!(target_arch = "x86_64") {
            assert_eq!(h.rust_target_arch(), "x86_64");
        }
        if cfg!(target_arch = "aarch64") {
            assert_eq!(h.rust_target_arch(), "aarch64");
        }
    }

    #[test]
    fn matches_platform_names() {
        let lx = p("linux/x64");
        assert!(lx.matches_name("@img/sharp-linux-x64"));
        assert!(lx.matches_name("@img/sharp-libvips-linux-x64"));
        assert!(!lx.matches_name("@img/sharp-linuxmusl-x64"));
        assert!(!lx.matches_name("@img/sharp-darwin-arm64"));
        assert!(!lx.matches_name("@img/sharp-linux-arm64"));
        assert!(!lx.matches_name("@img/sharp-wasm32"));
        assert!(lx.matches_name("@esbuild/linux-x64"));
        assert!(lx.matches_name("esbuild-linux-64"));
        assert!(!lx.matches_name("esbuild-windows-64"));
        assert!(lx.matches_name("@next/swc-linux-x64-gnu"));
        assert!(!lx.matches_name("@next/swc-linux-x64-musl"));
        assert!(lx.matches_name("node.napi.glibc.node"));
        assert!(!lx.matches_name("node.napi.musl.node"));
        assert!(lx.matches_name("left-pad"));
        let musl = p("linux/x64/musl");
        assert!(musl.matches_name("@img/sharp-linuxmusl-x64"));
        assert!(!musl.matches_name("@next/swc-linux-x64-gnu"));
        let mac = p("darwin/arm64");
        assert!(mac.matches_name("@esbuild/darwin-arm64"));
        assert!(mac.matches_name("fsevents-darwin-universal"));
    }

    #[test]
    fn sharedlibs() {
        let lx = p("linux/x64");
        assert!(lx.is_sharedlib(b"libvips-cpp.so.42"));
        assert!(lx.is_sharedlib(b"libfoo.so"));
        assert!(!lx.is_sharedlib(b"libfoo.dylib"));
        assert!(!lx.is_sharedlib(b"libquery_engine-debian-openssl-3.0.x.so.node"));
        assert!(!lx.is_sharedlib(b"x.json"));
        assert!(!lx.is_sharedlib(b"also.something"));
        let mac = p("darwin/arm64");
        assert!(mac.is_sharedlib(b"libvips-cpp.42.dylib"));
        assert!(mac.is_sharedlib(b"libfoo.so.1"));
        assert!(is_sharedlib_for(Os::Windows, b"foo.dll"));
        assert!(!is_sharedlib_for(Os::Windows, b"foo.so"));
    }

    #[test]
    fn python_names() {
        let lx = p("linux/x64");
        assert_eq!(lx.python_multiarch(), "x86_64-linux-gnu");
        assert_eq!(
            lx.python_ext_suffixes("cpython-312"),
            [".cpython-312-x86_64-linux-gnu.so", ".abi3.so", ".so"]
        );
        assert_eq!(
            p("linux/arm64/musl").python_multiarch(),
            "aarch64-linux-musl"
        );
        assert_eq!(p("linux/arm").python_multiarch(), "arm-linux-gnueabihf");
        assert_eq!(p("darwin/arm64").python_multiarch(), "darwin");
        assert_eq!(
            p("windows/x64").python_ext_suffixes("cpython-312"),
            [".cp312-win_amd64.pyd", ".pyd"]
        );
        assert!(lx.python_wheel_tag_matches("manylinux_2_17_x86_64.manylinux2014_x86_64"));
        assert!(lx.python_wheel_tag_matches("linux_x86_64"));
        assert!(lx.python_wheel_tag_matches("any"));
        assert!(!lx.python_wheel_tag_matches("musllinux_1_1_x86_64"));
        assert!(!lx.python_wheel_tag_matches("manylinux_2_17_aarch64"));
        assert!(!lx.python_wheel_tag_matches("macosx_11_0_x86_64"));
        assert!(p("linux/x64/musl").python_wheel_tag_matches("musllinux_1_2_x86_64"));
        let mac = p("darwin/arm64");
        assert!(mac.python_wheel_tag_matches("macosx_11_0_arm64"));
        assert!(mac.python_wheel_tag_matches("macosx_10_9_universal2"));
        assert!(!mac.python_wheel_tag_matches("macosx_10_9_x86_64"));
        assert!(p("windows/x64").python_wheel_tag_matches("win_amd64"));
        assert_eq!(mac.python_sys_platform(), "darwin");
    }

    #[test]
    fn go_names() {
        let lx = p("linux/amd64");
        assert_eq!((lx.go_os(), lx.go_arch()), ("linux", "amd64"));
        assert_eq!(lx.go_tag_matches("linux"), Some(true));
        assert_eq!(lx.go_tag_matches("unix"), Some(true));
        assert_eq!(lx.go_tag_matches("darwin"), Some(false));
        assert_eq!(lx.go_tag_matches("arm64"), Some(false));
        assert_eq!(lx.go_tag_matches("cgo"), None);
        assert!(lx.go_file_matches("x_linux.go"));
        assert!(lx.go_file_matches("x_linux_amd64_test.go"));
        assert!(!lx.go_file_matches("x_darwin.go"));
        assert!(!lx.go_file_matches("x_linux_arm64.go"));
        assert!(!lx.go_file_matches("x_arm64.go"));
        assert!(lx.go_file_matches("x.go"));
        assert!(lx.go_file_matches("linux.go"));
        assert!(lx.go_file_matches("x_custom.go"));
        assert_eq!(p("windows/386").go_arch(), "386");
    }

    #[test]
    fn rust_names() {
        assert_eq!(p("linux/x64").rust_triple(), "x86_64-unknown-linux-gnu");
        assert_eq!(
            p("linux/arm64/musl").rust_triple(),
            "aarch64-unknown-linux-musl"
        );
        assert_eq!(p("darwin/arm64").rust_triple(), "aarch64-apple-darwin");
        assert_eq!(p("windows/x64").rust_triple(), "x86_64-pc-windows-msvc");
        let mac = p("darwin/arm64");
        assert_eq!(mac.rust_cfg("target_os", Some("macos")), Some(true));
        assert_eq!(mac.rust_cfg("target_os", Some("linux")), Some(false));
        assert_eq!(mac.rust_cfg("unix", None), Some(true));
        assert_eq!(mac.rust_cfg("windows", None), Some(false));
        assert_eq!(mac.rust_cfg("target_pointer_width", Some("64")), Some(true));
        assert_eq!(mac.rust_cfg("feature", Some("x")), None);
        assert_eq!(
            p("linux/x64/musl").rust_cfg("target_env", Some("musl")),
            Some(true)
        );
    }
}
