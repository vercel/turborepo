//! Package special cases: packages that
//! locate native binaries or data files at runtime in ways static analysis
//! cannot see.
//!
//! Modeled on nft's `src/utils/special-cases.ts` and its binary locators in
//! `src/analyze.ts`, in the style of PyInstaller hooks: mostly data, plus small
//! hooks.
//!
//! Two triggers:
//! - Loader packages ([`LOADERS`]): a module reference to `bindings`,
//!   `node-gyp-build`, `node-gyp-build-optional-packages`,
//!   `@mapbox/node-pre-gyp`, `node-pre-gyp` or `nbind` from a file of package P
//!   searches P for the native addon the loader would pick at runtime (P is
//!   nft's `module_root`: the requiring file's package base). The addon name
//!   passed to the loader is not known to the resolver, so every candidate
//!   `.node` in the searched locations is included (over-approximation). Edges
//!   have kind [`RefKind::NativeAddon`](turborepo_trace_core::RefKind::NativeAddon),
//!   reason `asset`.
//! - Known files ([`FILE_CASES`], plus Prisma's generated client): when a
//!   specific file of a package is parsed, emit the data directories or files
//!   nft emits for it (`sharp`'s platform packages, `esbuild`'s binary package,
//!   `typescript`'s lib directory, ...).
//!
//! Native addons also pull in the shared libraries shipped in their package
//! (nft's `sharedlibEmit`, reason `sharedlib`); that is language-neutral and
//! lives in [`turborepo_trace_core::resolve_path::sharedlib`].
//!
//! The case format ([`FileCase`], [`Action`]) and the common actions (`Dir`,
//! `File`, `Dep`, `Glob`) are shared by all languages
//! ([`turborepo_trace_core::resolve_path::special`]). What stays
//! here is npm-specific: the table, the npm-only actions ([`NpmAction`]:
//! optional dependencies, another package's directory), the loader packages,
//! node-pre-gyp's `binary` config and Prisma's generated client.
//!
//! Every edge's spec starts with
//! [`SPECIAL_CASE_PREFIX`](turborepo_trace_core::output::SPECIAL_CASE_PREFIX)
//! so `--format summary` can list hits. Platform-dependent choices use
//! [`Platform`] (the tracer's `--target`, default host) in Node names.

mod pregyp;

use std::path::{Path, PathBuf};

pub use turborepo_trace_core::platform::{Arch, Libc, Os, Platform};
use turborepo_trace_core::{
    ExtraEdge, PackageBoundary, RefKind, ResolveCx, Target, Uncertainty, UncertaintyKind,
    fs::normalize,
    resolve_path::{
        GlobOut,
        assets::{keep_all, pattern_parts, special_spec as spec},
        special::{self as shared, CaseConfig},
    },
};

use crate::resolve::assets::{JS_ASSET_RULES, find_package_dir};

/// Loader packages whose import triggers a native-addon search in the importing
/// package.
pub const LOADERS: &[&str] = &[
    "bindings",
    "node-gyp-build",
    "@aminya/node-gyp-build",
    "node-gyp-build-optional-packages",
    "@mapbox/node-pre-gyp",
    "node-pre-gyp",
    "@discordjs/node-pre-gyp",
    "nbind",
];

/// npm-only actions of a file-triggered case.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NpmAction {
    /// Every file of each installed `optionalDependencies` package of the
    /// triggering package that matches the platform. With `nested`, also
    /// those packages' own optional dependencies (sharp's
    /// `@img/sharp-<platform>` → `@img/sharp-libvips-*`).
    OptionalDeps {
        /// Follow one more level.
        nested: bool,
    },
    /// Every file below `<dir>` of another package found from the triggering
    /// file.
    PackageDir(&'static str, &'static str),
}

/// What a file-triggered case emits (the shared format with [`NpmAction`]s).
/// Paths are relative to the triggering file's directory; `File`, `Dep` and
/// `Glob` paths may contain `{os}`, `{arch}` and `{libc_os}`.
pub type Action = shared::Action<NpmAction>;

/// A file-triggered case: package name, path suffix of the triggering file
/// (starting with the package name, except `shiki`), actions.
pub type FileCase = shared::FileCase<NpmAction>;

use NpmAction::{OptionalDeps, PackageDir};
use shared::Action::{Dep, Dir, File, Glob, Lang};

/// How the shared common actions become edges for JS.
const CASE_CONFIG: CaseConfig = CaseConfig {
    rules: JS_ASSET_RULES,
    dep_kind: RefKind::Require,
    glob_kind: RefKind::NativeAddon,
};

/// nft's special cases (`src/utils/special-cases.ts`), as data.
pub const FILE_CASES: &[FileCase] = &[
    FileCase {
        package: "@generated/photon",
        suffix: "@generated/photon/index.js",
        actions: &[Dir("runtime")],
    },
    FileCase {
        package: "@serialport/bindings-cpp",
        suffix: "@serialport/bindings-cpp/dist/index.js",
        actions: &[Dir("../build/Release"), Dir("../prebuilds")],
    },
    FileCase {
        package: "argon2",
        suffix: "argon2/argon2.js",
        actions: &[Dir("build/Release"), Dir("prebuilds"), Dir("lib/binding")],
    },
    FileCase {
        package: "argon2",
        suffix: "argon2/argon2.cjs",
        actions: &[Dir("build/Release"), Dir("prebuilds"), Dir("lib/binding")],
    },
    FileCase {
        package: "bull",
        suffix: "bull/lib/commands/index.js",
        actions: &[Dir(".")],
    },
    FileCase {
        package: "camaro",
        suffix: "camaro/dist/camaro.js",
        actions: &[File("camaro.wasm")],
    },
    FileCase {
        package: "esbuild",
        suffix: "esbuild/lib/main.js",
        actions: &[Lang(OptionalDeps { nested: false })],
    },
    FileCase {
        package: "ffmpeg-static",
        suffix: "ffmpeg-static/index.js",
        actions: &[File("ffmpeg"), File("bin/{os}/{arch}/ffmpeg")],
    },
    FileCase {
        package: "google-gax",
        suffix: "google-gax/build/src/grpc.js",
        actions: &[Dir("../../../google-proto-files")],
    },
    FileCase {
        package: "oracledb",
        suffix: "oracledb/lib/oracledb.js",
        actions: &[Glob("../build/Release/oracledb-*-{os}-{arch}.node")],
    },
    FileCase {
        package: "phantomjs-prebuilt",
        suffix: "phantomjs-prebuilt/lib/phantomjs.js",
        actions: &[Dir("../bin")],
    },
    FileCase {
        package: "remark-prism",
        suffix: "remark-prism/src/highlight.js",
        actions: &[Dir("../../prismjs/components")],
    },
    FileCase {
        package: "semver",
        suffix: "semver/index.js",
        actions: &[File("preload.js")],
    },
    FileCase {
        package: "sharp",
        suffix: "sharp/lib/index.js",
        actions: &[Lang(OptionalDeps { nested: true })],
    },
    // sharp >= 0.35 ships its entry points in `dist/`.
    FileCase {
        package: "sharp",
        suffix: "sharp/dist/index.cjs",
        actions: &[Lang(OptionalDeps { nested: true })],
    },
    FileCase {
        package: "sharp",
        suffix: "sharp/dist/index.mjs",
        actions: &[Lang(OptionalDeps { nested: true })],
    },
    // Ours: better-sqlite3 >= 12 locates its addon itself (`lib/binding.js`): a prebuild
    // for the platform, else node-gyp's build directories. Older versions use `bindings`.
    FileCase {
        package: "better-sqlite3",
        suffix: "better-sqlite3/lib/binding.js",
        actions: &[
            Glob("../prebuilds/{libc_os}-{arch}.node"),
            Glob("../build/Release/better_sqlite3.node"),
            Glob("../build/Debug/better_sqlite3.node"),
        ],
    },
    FileCase {
        package: "shiki",
        suffix: "/dist/index.js",
        actions: &[Dir("../languages"), Dir("../themes")],
    },
    FileCase {
        package: "socket.io",
        suffix: "socket.io/lib/index.js",
        actions: &[Lang(PackageDir("socket.io-client", "dist"))],
    },
    FileCase {
        package: "typescript",
        suffix: "typescript/lib/tsc.js",
        actions: &[Dir(".")],
    },
    FileCase {
        package: "uglify-es",
        suffix: "uglify-es/tools/node.js",
        actions: &[
            File("../lib/utils.js"),
            File("../lib/ast.js"),
            File("../lib/parse.js"),
            File("../lib/transform.js"),
            File("../lib/scope.js"),
            File("../lib/output.js"),
            File("../lib/compress.js"),
            File("../lib/sourcemap.js"),
            File("../lib/mozilla-ast.js"),
            File("../lib/propmangle.js"),
            File("../lib/minify.js"),
            File("exports.js"),
        ],
    },
    FileCase {
        package: "uglify-js",
        suffix: "uglify-js/tools/node.js",
        actions: &[Dir("../lib"), File("exports.js")],
    },
    FileCase {
        package: "playwright-core",
        suffix: "playwright-core/index.js",
        actions: &[File("browsers.json")],
    },
    FileCase {
        package: "geo-tz",
        suffix: "geo-tz/dist/geo-tz.js",
        actions: &[File("../data/geo.dat")],
    },
    FileCase {
        package: "pixelmatch",
        suffix: "pixelmatch/index.js",
        actions: &[Dep("bin/pixelmatch")],
    },
    FileCase {
        package: "geoip-lite",
        suffix: "geoip-lite/lib/geoip.js",
        actions: &[
            File("../data/geoip-city.dat"),
            File("../data/geoip-city6.dat"),
            File("../data/geoip-city-names.dat"),
            File("../data/geoip-country.dat"),
            File("../data/geoip-country6.dat"),
        ],
    },
];

/// `bindings`' search locations below the module root (`bindings.js`
/// `defaults.try`), with `{os}`/`{arch}` for the target and `*` for unknown
/// parts (Node version, ABI).
const BINDINGS_DIRS: &[&str] = &[
    "build",
    "build/Debug",
    "build/Release",
    "out/Debug",
    "Debug",
    "out/Release",
    "Release",
    "build/default",
    "compiled/*/{os}/{arch}",
    "addon-build/release/install-root",
    "addon-build/debug/install-root",
    "addon-build/default/install-root",
    "lib/binding/node-v*-{os}-{arch}",
];

/// The special-case engine for one resolver.
#[derive(Debug, Default)]
pub struct Special {
    platform: Platform,
}

/// nft's `getPackageBase` (see [`PackageBoundary::package_root`]).
fn package_base(path: &Path) -> Option<PathBuf> {
    PackageBoundary::NODE_MODULES.package_root(path)
}

/// Expands `{os}`, `{arch}` and `{libc_os}` (`linuxmusl` on musl, else the OS).
fn subst(s: &str, p: &Platform) -> String {
    let libc_os = if p.libc == Libc::Musl {
        "linuxmusl"
    } else {
        p.node_os()
    };
    s.replace("{os}", p.node_os())
        .replace("{arch}", p.node_arch())
        .replace("{libc_os}", libc_os)
}

fn read_json(cx: &ResolveCx<'_>, path: &Path) -> Option<serde_json::Value> {
    let bytes = cx.fs.read_cached(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The root directory of the package containing `file`: its `node_modules`
/// package base, or else the nearest directory with a `package.json`.
fn module_root(cx: &ResolveCx<'_>, file: &Path) -> Option<PathBuf> {
    if let Some(b) = package_base(file) {
        return Some(b);
    }
    file.ancestors()
        .skip(1)
        .find(|d| cx.fs.is_file(&d.join("package.json")))
        .map(Path::to_path_buf)
}

fn is_node_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "node")
}

impl Special {
    /// A special-case engine for `platform`.
    pub fn new(platform: Platform) -> Self {
        Special { platform }
    }

    /// The target platform.
    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    /// Edges for a module reference to loader package `loader` from
    /// `cx.from_path`, or nothing if `loader` is not in [`LOADERS`].
    pub fn loader_edges(&self, loader: &str, cx: &ResolveCx<'_>) -> Vec<ExtraEdge> {
        if !LOADERS.contains(&loader) {
            return Vec::new();
        }
        let Some(root) = module_root(cx, cx.from_path) else {
            return Vec::new();
        };
        // The loader's own files do not load addons for themselves.
        if root.ends_with(loader) {
            return Vec::new();
        }
        let p = &self.platform;
        let mut globs: Vec<String> = Vec::new();
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut partial: Option<String> = None;
        match loader {
            "bindings" => {
                globs.extend(
                    BINDINGS_DIRS
                        .iter()
                        .map(|d| format!("{}/*.node", subst(d, p))),
                );
            }
            "node-gyp-build" | "@aminya/node-gyp-build" => {
                globs.extend(self.gyp_build_globs());
            }
            "node-gyp-build-optional-packages" => {
                globs.extend(self.gyp_build_globs());
                dirs.extend(self.optional_dep_dirs(cx, &root, false));
            }
            "nbind" => {
                globs.extend(["build/Release/*.node".into(), "build/Debug/*.node".into()]);
            }
            _ => {
                // node-pre-gyp family: the `binary` config in the package's package.json.
                match read_json(cx, &root.join("package.json"))
                    .and_then(|pj| pregyp::module_pattern(&pj, p))
                {
                    Some(m) => {
                        if !m.unknown.is_empty() {
                            partial = Some(format!(
                                "{loader}: binary.module_path placeholders not known statically: \
                                 {}",
                                m.unknown.join(", ")
                            ));
                        }
                        globs.push(m.pattern);
                    }
                    None => {
                        // No usable `binary` config: search the conventional location.
                        globs.push("lib/binding/*.node".into());
                        partial = Some(format!(
                            "{loader}: no binary.module_name/module_path in {}; searched \
                             lib/binding",
                            root.join("package.json").display()
                        ));
                    }
                }
            }
        }

        let mut out = Vec::new();
        let mut matched = 0usize;
        let root_str = root.to_string_lossy();
        for g in &globs {
            let parts = pattern_parts(&format!("{root_str}/{g}"));
            let Some(found) = JS_ASSET_RULES.glob(&parts, Path::new("/"), cx, keep_all) else {
                continue;
            };
            // `*` in eval globs spans directories; the loaders look in exact directories.
            let depth = g.split('/').count();
            let found = self.only_platform(found, |f| {
                is_node_file(f)
                    && f.strip_prefix(&root)
                        .is_ok_and(|r| r.components().count() == depth)
            });
            if found.matches.is_empty() {
                continue;
            }
            matched += found.matches.len();
            let truncated = found.truncated;
            let mut e = ExtraEdge::new(
                RefKind::NativeAddon,
                found.into_target(cx),
                spec(format!("{loader}: {g}")),
            );
            if truncated {
                e = e.with_uncertainty(Uncertainty::new(
                    UncertaintyKind::PatternTooBroad,
                    turborepo_trace_core::Span::DUMMY,
                    format!("{loader}: more than {} matches", cx.options.max_glob),
                ));
            }
            out.push(e);
        }
        for d in dirs {
            let name = turborepo_trace_core::output::package_of(&d.join("x"))
                .unwrap_or_else(|| d.display().to_string());
            let g = JS_ASSET_RULES.dir_glob(&d, cx);
            if g.matches.is_empty() {
                continue;
            }
            matched += g.matches.len();
            out.push(ExtraEdge::new(
                RefKind::NativeAddon,
                g.into_target(cx),
                spec(format!("{loader}: optional package {name}")),
            ));
        }
        if matched == 0 {
            out.push(
                ExtraEdge::new(
                    RefKind::NativeAddon,
                    Target::Unresolved,
                    spec(format!("{loader}: native addon")),
                )
                .with_uncertainty(Uncertainty::new(
                    UncertaintyKind::UnresolvedSpecifier,
                    turborepo_trace_core::Span::DUMMY,
                    format!(
                        "{loader}: no native addon for {} found in {} (searched {})",
                        p.os_arch(),
                        root.display(),
                        globs.join(", ")
                    ),
                )),
            );
        } else if let Some(detail) = partial
            && let Some(first) = out.first_mut()
        {
            first.uncertainty.get_or_insert_with(|| {
                Uncertainty::new(
                    UncertaintyKind::PartialSpecifier,
                    turborepo_trace_core::Span::DUMMY,
                    detail,
                )
            });
        }
        out
    }

    /// Where `node-gyp-build` looks: `build/Release`, `build/Debug`, then
    /// `prebuilds/<os>-<arch>/`.
    fn gyp_build_globs(&self) -> Vec<String> {
        let mut v = vec![
            "build/Release/*.node".into(),
            "build/Debug/*.node".into(),
            format!("prebuilds/{}/*.node", self.platform.os_arch()),
        ];
        // prebuildify's libc-tagged directories (`prebuilds/linuxglibc-x64`,
        // `prebuilds/linuxmusl-arm64`), used by `@datadog/pprof` and others.
        let libc = match self.platform.libc {
            Libc::Glibc => Some("glibc"),
            Libc::Musl => Some("musl"),
            _ => None,
        };
        if let Some(libc) = libc {
            v.push(format!(
                "prebuilds/{}{libc}-{}/*.node",
                self.platform.node_os(),
                self.platform.node_arch()
            ));
        }
        v
    }

    /// Drops matches whose file names are for another platform or libc
    /// (`node.napi.musl.node` on glibc) or that fail `keep`.
    fn only_platform(&self, mut g: GlobOut, keep: impl Fn(&Path) -> bool) -> GlobOut {
        g.matches.retain(|m| {
            keep(m)
                && m.file_name()
                    .is_some_and(|n| self.platform.matches_name(&n.to_string_lossy()))
        });
        g
    }

    /// Installed `optionalDependencies` of the package at `root` that match the
    /// platform, as package directories (siblings in the same
    /// `node_modules`, or nested).
    fn optional_dep_dirs(&self, cx: &ResolveCx<'_>, root: &Path, nested: bool) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for (dir, _) in self.optional_deps_of(cx, root) {
            if !out.contains(&dir) {
                out.push(dir.clone());
            }
            if nested {
                // nft: resolve(realpath(dir), '..', '..', innerDep).
                let real = cx.fs.canonicalize(&dir).path;
                for (inner, _) in self.optional_deps_of(cx, &real) {
                    if !out.contains(&inner) {
                        out.push(inner);
                    }
                }
            }
        }
        out
    }

    /// `(dir, name)` of each installed optional dependency of the package at
    /// `root` whose name matches the platform. Dependencies are looked up
    /// next to the package (`<root>/../<name>`, scoped roots one level
    /// higher) and below it.
    fn optional_deps_of(&self, cx: &ResolveCx<'_>, root: &Path) -> Vec<(PathBuf, String)> {
        let Some(pj) = read_json(cx, &root.join("package.json")) else {
            return Vec::new();
        };
        let Some(deps) = pj.get("optionalDependencies").and_then(|d| d.as_object()) else {
            return Vec::new();
        };
        let name = pj.get("name").and_then(|n| n.as_str()).unwrap_or_default();
        let nm = if name.starts_with('@') {
            root.parent().and_then(Path::parent)
        } else {
            root.parent()
        };
        let mut out = Vec::new();
        for dep in deps.keys() {
            if !self.platform.matches_name(dep) {
                continue;
            }
            let cands = [
                nm.map(|nm| nm.join(dep)),
                Some(root.join("node_modules").join(dep)),
            ];
            if let Some(d) = cands.into_iter().flatten().find(|d| cx.fs.is_dir(d)) {
                out.push((normalize(&d), dep.clone()));
            }
        }
        out.sort();
        out
    }

    /// File-triggered edges for the parsed file `cx.from_path`.
    pub fn file_edges(&self, cx: &ResolveCx<'_>) -> Vec<ExtraEdge> {
        let file = cx.from_path;
        let mut out = Vec::new();
        let pkg = turborepo_trace_core::output::package_of(file);
        let path_str = file.to_string_lossy();
        let subst = |s: &str| subst(s, &self.platform);
        for case in FILE_CASES {
            if !case.matches(pkg.as_deref(), &path_str) {
                continue;
            }
            for a in case.actions {
                if let Some(npm) =
                    shared::apply(a, case.package, &CASE_CONFIG, &subst, cx, &mut out)
                {
                    self.npm_action(case.package, npm, cx, &mut out);
                }
            }
        }
        self.prisma(cx, &mut out);
        out
    }

    fn npm_action(&self, pkg: &str, a: &NpmAction, cx: &ResolveCx<'_>, out: &mut Vec<ExtraEdge>) {
        match *a {
            OptionalDeps { nested } => {
                // The triggering package's root (`<root>/lib/index.js`).
                let Some(root) = package_base(cx.from_path) else {
                    return;
                };
                for d in self.optional_dep_dirs(cx, &root, nested) {
                    let name = turborepo_trace_core::output::package_of(&d.join("x"))
                        .unwrap_or_else(|| d.display().to_string());
                    out.push(dir_edge(
                        &d,
                        cx,
                        format!("{pkg}: optional dependency {name}"),
                    ));
                }
            }
            PackageDir(other, sub) => {
                if let Some(d) = find_package_dir(cx, cx.from_dir, other) {
                    let d = d.join(sub);
                    if cx.fs.is_dir(&d) {
                        out.push(dir_edge(&d, cx, format!("{pkg}: {other}/{sub}/")));
                    }
                }
            }
        }
    }

    /// Prisma's generated client (`node_modules/.prisma/client`, or a custom
    /// `output` directory): a parsed file next to `schema.prisma` and query
    /// engine files emits the schema and the engines for the target
    /// platform. Prisma's own code references them with literal
    /// `path.join(__dirname, ...)` calls, but only for the platform it was
    /// generated on and behind environment checks.
    fn prisma(&self, cx: &ResolveCx<'_>, out: &mut Vec<ExtraEdge>) {
        let Some(name) = cx.from_path.file_name().and_then(|n| n.to_str()) else {
            return;
        };
        if !matches!(
            name,
            "index.js" | "default.js" | "client.js" | "index.mjs" | "wasm.js" | "edge.js"
        ) {
            return;
        }
        let Some(listing) = cx.fs.list(cx.from_dir) else {
            return;
        };
        if listing.get(b"schema.prisma").is_none() {
            return;
        }
        let engines: Vec<String> = listing
            .iter()
            .filter_map(|e| e.name().to_str().map(String::from))
            .filter(|n| n.starts_with("libquery_engine-") || n.starts_with("query_engine-"))
            .collect();
        let schema = cx.from_dir.join("schema.prisma");
        out.push(ExtraEdge::new(
            RefKind::Asset,
            Target::File(cx.intern(&schema)),
            spec("prisma: schema.prisma"),
        ));
        if engines.is_empty() {
            return;
        }
        let p = &self.platform;
        let wanted: Vec<&String> = engines
            .iter()
            .filter(|n| {
                let lib_ok = match p.node_os() {
                    "darwin" => n.ends_with(".dylib.node") || n.starts_with("query_engine-darwin"),
                    "linux" => {
                        n.ends_with(".so.node")
                            || (n.starts_with("query_engine-") && !n.contains("darwin"))
                    }
                    _ => true,
                };
                let arm = n.contains("arm64") || n.contains("aarch64");
                let musl = n.contains("musl");
                lib_ok
                    && arm == (p.node_arch() == "arm64")
                    && (p.os != Os::Linux || musl == (p.libc == Libc::Musl))
            })
            .collect();
        if wanted.is_empty() {
            out.push(
                ExtraEdge::new(
                    RefKind::NativeAddon,
                    Target::Unresolved,
                    spec("prisma: query engine"),
                )
                .with_uncertainty(Uncertainty::new(
                    UncertaintyKind::UnresolvedSpecifier,
                    turborepo_trace_core::Span::DUMMY,
                    format!(
                        "prisma: no query engine for {} in {} (found: {}); add the binaryTarget \
                         to schema.prisma",
                        p.os_arch(),
                        cx.from_dir.display(),
                        engines.join(", ")
                    ),
                )),
            );
            return;
        }
        for n in wanted {
            out.push(ExtraEdge::new(
                RefKind::NativeAddon,
                Target::File(cx.intern(&cx.from_dir.join(n))),
                spec(format!("prisma: {n}")),
            ));
        }
    }
}

/// An asset-directory edge.
fn dir_edge(dir: &Path, cx: &ResolveCx<'_>, label: String) -> ExtraEdge {
    JS_ASSET_RULES.dir_edge(dir, cx, label)
}
