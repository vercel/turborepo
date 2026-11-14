//! A synthetic large pnpm/turbo monorepo for graph-store scale benchmarks.
//!
//! [`Monorepo::generate_in`] writes a deterministic tree shaped like a big JS
//! monorepo:
//!
//! - `packages/pNNNN` and `apps/aNNNN` workspace packages (`@syn/…`), each with
//!   a `package.json` (scripts: `build`, `test`, `typecheck`, some `codegen`
//!   and `prisma generate`), a `jest.config.js` that requires the root config,
//!   a `tsconfig.json`, `src/*.ts` sources with relative, workspace (`@syn/pX`,
//!   `@syn/pX/src/fK`) and third-party imports, `src/__tests__/*.test.ts` jest
//!   tests with `jest.mock`, and some `__mocks__`;
//! - a root package with `turbo.json` (outputs `dist/**` and
//!   `src/generated/**`), the root `jest.config.js`, `tsconfig.base.json`,
//!   `pnpm-workspace.yaml`, root `__mocks__/` and `scripts/`;
//! - a `pnpm-lock.yaml` with every importer, and an installed pnpm layout
//!   (`node_modules/.pnpm/<name>@<version>/node_modules/<name>` plus
//!   per-importer symlinks) for the third-party packages, `@prisma/client`
//!   included (an output package of `prisma generate`);
//! - imports of `./generated/schema` in `codegen` packages: gitignored
//!   generated files that a task output covers.
//!
//! The defaults describe a large monorepo at full scale (1500 packages, about
//! 50k files); [`Monorepo::scaled`] divides that.

use std::{fmt::Write as _, fs, io, path::Path};

use super::Rng;

/// Shape of the generated monorepo.
#[derive(Clone, Debug)]
pub struct Monorepo {
    /// Workspace packages (about 10% are apps).
    pub packages: usize,
    /// Source files per package (tests included).
    pub files_per_package: usize,
    /// Third-party packages in the lockfile.
    pub third_party: usize,
    /// Write the installed `node_modules` layout (otherwise lockfile only).
    pub install: bool,
    /// Gitignored build-output directories per package (`dist/chunks/c<k>/`).
    pub dist_dirs: usize,
    /// Give about one package in 15 a `knip --directory ../..` script: the same
    /// repo-wide membership hub in many packages.
    pub hubs: bool,
    /// Seed.
    pub seed: u64,
}

impl Default for Monorepo {
    fn default() -> Self {
        Monorepo {
            packages: 1500,
            files_per_package: 33,
            third_party: 1000,
            install: true,
            dist_dirs: 4,
            hubs: false,
            seed: 7,
        }
    }
}

/// Sets the mtime of every file and directory below `root` (symlinks excluded)
/// to `secs` seconds ago, so a freshly generated tree is not too recent to
/// trust for reuse.
pub fn backdate(root: &Path, secs: u64) -> io::Result<()> {
    let t = std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
    fn walk(p: &Path, t: std::time::SystemTime) -> io::Result<()> {
        let md = fs::symlink_metadata(p)?;
        if md.file_type().is_symlink() {
            return Ok(());
        }
        if md.is_dir() {
            for e in fs::read_dir(p)? {
                walk(&e?.path(), t)?;
            }
        }
        // Directories last: listing them does not change their mtime, setting children
        // does not either, but keep the order obvious.
        fs::File::open(p)?.set_modified(t)
    }
    walk(root, t)
}

/// What was generated.
#[derive(Clone, Debug, Default)]
pub struct MonorepoStats {
    /// Files written (symlinks excluded).
    pub files: usize,
    /// Source files (`.ts`) in workspace packages.
    pub sources: usize,
    /// Jest test files.
    pub tests: usize,
    /// Repo-relative path of one source file, a good single-file edit.
    pub sample_source: String,
    /// Repo-relative paths of source files spread over the repository.
    pub spread_sources: Vec<String>,
}

/// One workspace package.
struct Pkg {
    dir: String,
    name: String,
    ws_deps: Vec<usize>,
    tp_deps: Vec<usize>,
    codegen: bool,
    prisma: bool,
    sources: usize,
    tests: usize,
}

impl Monorepo {
    /// The default shape divided by `div` (at least 4 packages and 4 files per
    /// package).
    pub fn scaled(div: usize) -> Self {
        let d = Monorepo::default();
        let div = div.max(1);
        Monorepo {
            packages: (d.packages / div).max(4),
            third_party: (d.third_party / div).max(8),
            ..d
        }
    }

    fn tp_name(k: usize) -> String {
        if k == 0 {
            "@prisma/client".to_owned()
        } else if k.is_multiple_of(5) {
            format!("@tp/lib{k}")
        } else {
            format!("tp-lib{k}")
        }
    }

    fn tp_version(k: usize) -> String {
        format!("{}.{}.0", 1 + k % 3, k % 10)
    }

    fn plan(&self) -> Vec<Pkg> {
        let mut rng = Rng::new(self.seed);
        let tp = self.third_party.max(2);
        (0..self.packages)
            .map(|i| {
                let app = i % 10 == 9;
                let dir = if app {
                    format!("apps/a{i:04}")
                } else {
                    format!("packages/p{i:04}")
                };
                let name = if app {
                    format!("@syn/a{i:04}")
                } else {
                    format!("@syn/p{i:04}")
                };
                let mut ws_deps: Vec<usize> = if i == 0 {
                    Vec::new()
                } else {
                    // Mostly nearby lower packages, sometimes a popular low-index one.
                    (0..1 + rng.below(4))
                        .map(|_| {
                            if rng.below(3) == 0 {
                                rng.below(i.min(20))
                            } else {
                                i - 1 - rng.below(i.min(50))
                            }
                        })
                        .collect()
                };
                ws_deps.sort_unstable();
                ws_deps.dedup();
                let prisma = i % 23 == 4;
                let mut tp_deps: Vec<usize> = (0..2 + rng.below(4))
                    .map(|_| 1 + rng.below(tp - 1))
                    .collect();
                if prisma {
                    tp_deps.push(0);
                }
                tp_deps.sort_unstable();
                tp_deps.dedup();
                let files = self.files_per_package.max(4);
                let tests = (files / 5).max(1);
                Pkg {
                    dir,
                    name,
                    ws_deps,
                    tp_deps,
                    codegen: i % 7 == 3,
                    prisma,
                    sources: files - tests,
                    tests,
                }
            })
            .collect()
    }

    /// The workspace packages (directory) with the directories of their
    /// workspace dependencies, as generated.
    pub fn workspace_deps(&self) -> Vec<(String, Vec<String>)> {
        let pkgs = self.plan();
        pkgs.iter()
            .map(|p| {
                (
                    p.dir.clone(),
                    p.ws_deps.iter().map(|&d| pkgs[d].dir.clone()).collect(),
                )
            })
            .collect()
    }

    /// Generates the monorepo into the empty directory `root`.
    pub fn generate_in(&self, root: &Path) -> io::Result<MonorepoStats> {
        let pkgs = self.plan();
        let mut st = MonorepoStats::default();
        let mut rng = Rng::new(self.seed ^ 0xA5A5);
        let put = |rel: &str, body: &str, st: &mut MonorepoStats| -> io::Result<()> {
            let p = root.join(rel);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d)?;
            }
            st.files += 1;
            fs::write(p, body)
        };

        // Root package.
        put(
            ".gitignore",
            "node_modules\n.turbotrace\ndist\ngenerated/\n",
            &mut st,
        )?;
        put(
            "package.json",
            "{\n  \"name\": \"syn-root\",\n  \"private\": true,\n  \"scripts\": {\n    \"test\": \
             \"turbo run test\",\n    \"build\": \"turbo run build\",\n    \"lint\": \"node \
             scripts/lint.js\"\n  },\n  \"devDependencies\": {\n    \"tp-lib1\": \"1.1.0\"\n  \
             }\n}\n",
            &mut st,
        )?;
        put(
            "pnpm-workspace.yaml",
            "packages:\n  - 'packages/*'\n  - 'apps/*'\n",
            &mut st,
        )?;
        put(
            "turbo.json",
            r#"{
  "$schema": "https://turbo.build/schema.json",
  "tasks": {
    "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
    "codegen": { "outputs": ["src/generated/**"] },
    "generate": { "cache": false },
    "test": { "dependsOn": ["^build"], "inputs": ["src/**", "jest.config.js"] },
    "typecheck": { "dependsOn": ["^build"] }
  }
}
"#,
            &mut st,
        )?;
        put(
            "jest.config.js",
            "// Root jest config, extended by every package.\nmodule.exports = {\n  transform: { \
             '^.+\\\\.(t|j)sx?$': '@swc/jest' },\n  modulePathIgnorePatterns: \
             ['<rootDir>/.*/__mocks__'],\n  setupFilesAfterEnv: [],\n};\n",
            &mut st,
        )?;
        put(
            "tsconfig.base.json",
            "{\n  \"compilerOptions\": { \"strict\": true, \"module\": \"commonjs\", \"target\": \
             \"es2022\" }\n}\n",
            &mut st,
        )?;
        put(
            "__mocks__/tp-lib1.ts",
            "export const mocked = true;\n",
            &mut st,
        )?;
        for k in 0..8 {
            put(
                &format!("scripts/s{k}.js"),
                &format!(
                    "const path = require('path');\nconst fs = \
                     require('fs');\nrequire('./s{}');\nmodule.exports = () => \
                     path.join(__dirname, 'x{k}');\n",
                    (k + 1) % 8
                ),
                &mut st,
            )?;
        }
        put("scripts/lint.js", "require('./s0');\n", &mut st)?;

        // Workspace packages.
        for (i, p) in pkgs.iter().enumerate() {
            let up = "../..";
            let mut deps = String::new();
            for &d in &p.ws_deps {
                let _ = writeln!(deps, "    \"{}\": \"workspace:*\",", pkgs[d].name);
            }
            for &k in &p.tp_deps {
                let _ = writeln!(
                    deps,
                    "    \"{}\": \"{}\",",
                    Self::tp_name(k),
                    Self::tp_version(k)
                );
            }
            let deps = deps.trim_end().trim_end_matches(',');
            let mut scripts = String::from(
                "    \"build\": \"tsc -p tsconfig.json\",\n    \"test\": \"jest\",\n    \
                 \"typecheck\": \"tsc --noEmit\"",
            );
            if p.codegen {
                scripts.push_str(",\n    \"codegen\": \"node scripts/codegen.js\"");
            }
            if p.prisma {
                scripts.push_str(",\n    \"generate\": \"prisma generate\"");
            }
            if self.hubs && i % 15 == 6 {
                // A repo-wide membership hub repeated across packages (knip lists the whole
                // repository from every package that runs it).
                scripts.push_str(&format!(
                    ",\n    \"lint:unused\": \"knip --directory {up} --workspace {}\"",
                    p.dir
                ));
            }
            put(
                &format!("{}/package.json", p.dir),
                &format!(
                    "{{\n  \"name\": \"{}\",\n  \"version\": \"1.0.0\",\n  \"private\": true,\n  \
                     \"main\": \"./src/index.ts\",\n  \"scripts\": {{\n{scripts}\n  }},\n  \
                     \"dependencies\": {{\n{deps}\n  }}\n}}\n",
                    p.name
                ),
                &mut st,
            )?;
            put(
                &format!("{}/jest.config.js", p.dir),
                &format!(
                    "const rootConfig = require('{up}/jest.config.js');\n\nmodule.exports = {{\n  \
                     ...rootConfig,\n}};\n"
                ),
                &mut st,
            )?;
            put(
                &format!("{}/tsconfig.json", p.dir),
                &format!(
                    "{{\n  \"extends\": \"{up}/tsconfig.base.json\",\n  \"include\": \
                     [\"src\"]\n}}\n"
                ),
                &mut st,
            )?;
            // Build output (gitignored, outside the walk): what a deep listing enters.
            for c in 0..self.dist_dirs {
                put(
                    &format!("{}/dist/chunks/c{c}/index.js", p.dir),
                    "module.exports = {};\n",
                    &mut st,
                )?;
            }
            if p.codegen {
                put(
                    &format!("{}/scripts/codegen.js", p.dir),
                    "const fs = require('fs');\nfs.writeFileSync('src/generated/schema.ts', \
                     'export {}');\n",
                    &mut st,
                )?;
            }
            if p.prisma {
                put(
                    &format!("{}/prisma/schema.prisma", p.dir),
                    "generator client {\n  provider = \"prisma-client-js\"\n}\n\ndatasource db \
                     {\n  provider = \"postgresql\"\n  url      = env(\"DATABASE_URL\")\n}\n",
                    &mut st,
                )?;
                put(
                    &format!("{}/src/db.ts", p.dir),
                    "import { PrismaClient } from '@prisma/client';\nexport const db = new \
                     PrismaClient();\n",
                    &mut st,
                )?;
            }
            if i % 9 == 2 {
                let k = p.tp_deps.last().copied().unwrap_or(1);
                put(
                    &format!("{}/__mocks__/{}.ts", p.dir, Self::tp_name(k)),
                    "export default {};\n",
                    &mut st,
                )?;
            }
            // Sources.
            let mut index = String::new();
            for k in 0..p.sources.min(6) {
                let _ = writeln!(index, "export * from './f{k}';");
            }
            put(&format!("{}/src/index.ts", p.dir), &index, &mut st)?;
            st.sources += 1;
            for k in 0..p.sources {
                let mut s = String::new();
                for _ in 0..3.min(k) {
                    let j = k - 1 - rng.below(k.min(8));
                    let _ = writeln!(s, "import * as l{j} from './f{j}';");
                }
                if !p.ws_deps.is_empty() && rng.below(2) == 0 {
                    let d = &pkgs[p.ws_deps[rng.below(p.ws_deps.len())]];
                    if rng.below(3) == 0 {
                        let f = rng.below(d.sources.max(1));
                        let _ = writeln!(s, "import * as w from '{}/src/f{f}';", d.name);
                    } else {
                        let _ = writeln!(s, "import * as w from '{}';", d.name);
                    }
                }
                if rng.below(2) == 0 {
                    let t = p.tp_deps[rng.below(p.tp_deps.len())];
                    let _ = writeln!(s, "import tp from '{}';", Self::tp_name(t));
                }
                if p.codegen && k % 4 == 1 {
                    let _ = writeln!(s, "import {{ schema }} from './generated/schema';");
                }
                if k % 11 == 5 {
                    let _ = writeln!(s, "const lazy = () => import('./f{}');", k / 2);
                }
                let _ = write!(
                    s,
                    "\nexport function fn{k}(x: number): number {{\n  // Some body text so files \
                     have realistic sizes.\n  const y = x * {k} + 1;\n  return y > 100 ? y - 100 \
                     : y;\n}}\n\nexport const value{k} = fn{k}({});\n",
                    k + i
                );
                let rel = format!("{}/src/f{k}.ts", p.dir);
                if i == pkgs.len() / 2 && k == p.sources / 2 {
                    st.sample_source = rel.clone();
                }
                if k == 1 && i % (pkgs.len() / 8).max(1) == 0 {
                    st.spread_sources.push(rel.clone());
                }
                put(&rel, &s, &mut st)?;
                st.sources += 1;
            }
            for k in 0..p.tests {
                let a = rng.below(p.sources);
                let b = rng.below(p.sources);
                let mut s = format!(
                    "import {{ fn{a} }} from '../f{a}';\nimport {{ fn{b} }} from '../f{b}';\n"
                );
                if k.is_multiple_of(3) {
                    let _ = writeln!(s, "jest.mock('../f{b}');");
                }
                let _ = write!(
                    s,
                    "\ndescribe('f{a}', () => {{\n  it('works', () => {{\n    expect(fn{a}(1) + \
                     fn{b}(2)).toBeGreaterThan(0);\n  }});\n}});\n"
                );
                put(
                    &format!("{}/src/__tests__/t{k}.test.ts", p.dir),
                    &s,
                    &mut st,
                )?;
                st.tests += 1;
            }
        }

        // Lockfile.
        let mut lock = String::from(
            "lockfileVersion: '9.0'\n\nsettings:\n  autoInstallPeers: true\n  \
             excludeLinksFromLockfile: false\n\nimporters:\n\n  .:\n    devDependencies:\n      \
             tp-lib1:\n        specifier: 1.1.0\n        version: 1.1.0\n",
        );
        for p in &pkgs {
            let _ = write!(lock, "\n  {}:\n    dependencies:\n", p.dir);
            let mut entries: Vec<(String, String, String)> = Vec::new();
            for &d in &p.ws_deps {
                let rel = format!("../../{}", pkgs[d].dir);
                entries.push((
                    pkgs[d].name.clone(),
                    "workspace:*".into(),
                    format!("link:{rel}"),
                ));
            }
            for &k in &p.tp_deps {
                entries.push((Self::tp_name(k), Self::tp_version(k), Self::tp_version(k)));
            }
            entries.sort();
            for (n, spec, v) in entries {
                let _ = write!(
                    lock,
                    "      '{n}':\n        specifier: {spec}\n        version: {v}\n"
                );
            }
        }
        lock.push_str("\npackages:\n\n");
        let tp = self.third_party.max(2);
        for k in 0..tp {
            let _ = write!(
                lock,
                "  '{}@{}':\n    resolution: {{integrity: sha512-syn{k:08}}}\n\n",
                Self::tp_name(k),
                Self::tp_version(k)
            );
        }
        lock.push_str("snapshots:\n\n");
        for k in 0..tp {
            let _ = write!(
                lock,
                "  '{}@{}': {{}}\n\n",
                Self::tp_name(k),
                Self::tp_version(k)
            );
        }
        put("pnpm-lock.yaml", &lock, &mut st)?;

        // Installed layout.
        if self.install {
            let store_dir = |k: usize| {
                format!(
                    "node_modules/.pnpm/{}@{}/node_modules/{}",
                    Self::tp_name(k).replace('/', "+"),
                    Self::tp_version(k),
                    Self::tp_name(k)
                )
            };
            for k in 0..tp {
                let d = store_dir(k);
                put(
                    &format!("{d}/package.json"),
                    &format!(
                        "{{\"name\":\"{}\",\"version\":\"{}\",\"main\":\"index.js\"}}\n",
                        Self::tp_name(k),
                        Self::tp_version(k)
                    ),
                    &mut st,
                )?;
                put(&format!("{d}/index.js"), "module.exports = {};\n", &mut st)?;
            }
            let link = |importer: &str, name: &str, target: &str| -> io::Result<()> {
                let l = if importer.is_empty() {
                    root.join("node_modules").join(name)
                } else {
                    root.join(importer).join("node_modules").join(name)
                };
                fs::create_dir_all(l.parent().expect("parent"))?;
                let depth = l
                    .parent()
                    .expect("parent")
                    .strip_prefix(root)
                    .expect("under root")
                    .components()
                    .count();
                let rel = format!("{}{target}", "../".repeat(depth));
                turborepo_trace_core::os::symlink(rel, l)
            };
            link("", "tp-lib1", &store_dir(1))?;
            for p in &pkgs {
                for &k in &p.tp_deps {
                    link(&p.dir, &Self::tp_name(k), &store_dir(k))?;
                }
                for &d in &p.ws_deps {
                    link(&p.dir, &pkgs[d].name, &pkgs[d].dir)?;
                }
            }
        }
        if st.sample_source.is_empty() {
            st.sample_source = format!("{}/src/f1.ts", pkgs[0].dir);
        }
        Ok(st)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_complete() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let m = Monorepo {
            packages: 12,
            files_per_package: 6,
            third_party: 10,
            ..Default::default()
        };
        let sa = m.generate_in(a.path()).unwrap();
        let sb = m.generate_in(b.path()).unwrap();
        assert_eq!(sa.files, sb.files);
        assert!(a.path().join(&sa.sample_source).is_file());
        let read = |r: &Path, p: &str| fs::read_to_string(r.join(p)).unwrap();
        assert_eq!(
            read(a.path(), "pnpm-lock.yaml"),
            read(b.path(), "pnpm-lock.yaml")
        );
        assert_eq!(
            read(a.path(), &sa.sample_source),
            read(b.path(), &sb.sample_source)
        );
        assert!(
            a.path()
                .join("packages/p0004/node_modules/@prisma/client/package.json")
                .is_file()
        );
    }
}
