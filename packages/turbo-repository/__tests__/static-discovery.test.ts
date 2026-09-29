import { describe, it, type TestContext } from "node:test";
import { strict as assert } from "node:assert";
import { execFile } from "node:child_process";
import {
  chmod,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile
} from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const binding = pathToFileURL(
  path.resolve(__dirname, "../js/dist/index.js")
).href;
const nativeFlags = {
  experimentalCargoWorkspaces: true,
  experimentalPythonWorkspaces: true,
  experimentalGoWorkspaces: true
};

async function write(root: string, file: string, contents: string) {
  const target = path.join(root, file);
  await mkdir(path.dirname(target), { recursive: true });
  await writeFile(target, contents);
}

async function fixture(t: TestContext, mixed = true) {
  const dir = await realpath(
    await mkdtemp(path.join(tmpdir(), "turbo-static-discovery-"))
  );
  t.after(() => rm(dir, { recursive: true, force: true }));
  const root = path.join(dir, "repo");
  await write(
    root,
    "package.json",
    JSON.stringify({
      name: "root-not-a-package",
      private: true,
      packageManager: "npm@10.9.2",
      workspaces: ["apps/*", "packages/*", "crates/*"]
    })
  );
  await write(
    root,
    "package-lock.json",
    JSON.stringify({
      name: "root-not-a-package",
      lockfileVersion: 3,
      packages: {}
    })
  );
  await write(
    root,
    "turbo.json",
    JSON.stringify({ futureFlags: mixed ? nativeFlags : {}, tasks: {} })
  );
  for (const [relativePath, manifest] of [
    ["packages/unrelated", { name: "unrelated", version: "1.0.0" }],
    [
      "packages/middle",
      { name: "a-middle", version: "1.0.0", dependencies: { "z-base": "*" } }
    ],
    ["packages/base", { name: "z-base", version: "1.0.0" }],
    [
      "apps/web",
      { name: "web", version: "1.0.0", devDependencies: { "a-middle": "*" } }
    ]
  ] as const) {
    await write(root, `${relativePath}/package.json`, JSON.stringify(manifest));
  }
  if (mixed) {
    await write(
      root,
      "Cargo.toml",
      '[workspace]\nmembers = ["crates/*"]\nresolver = "2"\n[workspace.metadata]\nname = "rust-aggregate"\n'
    );
    // Intentionally no src/lib.rs, src/main.rs, or explicit Cargo targets.
    await write(
      root,
      "crates/core/Cargo.toml",
      '[package]\nname = "core"\nversion = "0.1.0"\nedition = "2021"\n'
    );
    await write(
      root,
      "pyproject.toml",
      '[tool.turbo]\nname = "python-aggregate"\n[tool.uv.workspace]\nmembers = ["python/*"]\n'
    );
    await write(
      root,
      "python/api/pyproject.toml",
      '[project]\nname = "py-api"\nversion = "0.1.0"\nrequires-python = ">=3.11"\n'
    );
    await write(root, "go.work", "go 1.22.0\n\nuse (\n  ./go/service\n)\n");
    await write(
      root,
      "go/service/go.mod",
      "module example.com/service\n\ngo 1.22.0\n"
    );
  }
  return { dir, root };
}

type PathMode = "absent" | "empty" | "trap";

async function runIsolated(
  dir: string,
  root: string,
  body: string,
  mode: PathMode = "empty",
  cwd = root
) {
  const env = Object.fromEntries(
    Object.entries(process.env).filter(
      ([key]) =>
        !/^(PATH|NODE_OPTIONS|NODE_PATH|CARGO|RUSTC|RUSTUP|UV|PYTHON|GO)(_|$)/i.test(
          key
        ) && !/^(GOROOT|GOPATH|GOTOOLCHAIN|GOENV|GOFLAGS)$/i.test(key)
    )
  );
  const bin = path.join(dir, `bin-${mode}`);
  await mkdir(bin, { recursive: true });
  const log = path.join(dir, "tool-invocations.log");
  await writeFile(log, "");
  if (mode !== "absent") env.PATH = bin;
  if (mode === "trap") {
    // An absolute shebang also works with an otherwise empty PATH. Logging
    // before failure catches discovery that swallows a failed tool invocation.
    for (const tool of [
      "cargo",
      "rustc",
      "rustup",
      "uv",
      "python",
      "python3",
      "go",
      "which"
    ]) {
      const executable = path.join(bin, tool);
      const script = path.join(bin, `${tool}.cjs`);
      await writeFile(
        script,
        `require("node:fs").appendFileSync(${JSON.stringify(log)}, ${JSON.stringify(tool + "\n")});\nprocess.exit(97);\n`
      );
      const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
      await writeFile(
        executable,
        `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(script)}\n`
      );
      await chmod(executable, 0o755);
    }
    await assert.rejects(
      () => execFileAsync(path.join(bin, "cargo"), [], { env }),
      { code: 97 }
    );
    assert.equal(await readFile(log, "utf8"), "cargo\n");
    await writeFile(log, "");
  }
  const script = `
    import { strict as assert } from "node:assert";
    import * as path from "node:path";
    import { readFileSync } from "node:fs";
    import { StaticWorkspace, Workspace } from ${JSON.stringify(binding)};
    const root = ${JSON.stringify(root)};
    const log = ${JSON.stringify(log)};
    const pkg = (name, relativePath, toolchain = "javascript", manifest = "package.json") => ({
      name, absolutePath: path.join(root, relativePath),
      relativePath: path.normalize(relativePath),
      manifestPath: path.join(relativePath, manifest), toolchain
    });
    const jsPackages = [
      pkg("web", "apps/web"), pkg("z-base", "packages/base"),
      pkg("a-middle", "packages/middle"), pkg("unrelated", "packages/unrelated")
    ];
    const mixedPackages = [
      jsPackages[0], pkg("core", "crates/core", "rust", "Cargo.toml"),
      pkg("service", "go/service", "go", "go.mod"),
      ...jsPackages.slice(1), pkg("py-api", "python/api", "python", "pyproject.toml")
    ];
    ${body}
  `;
  try {
    const { stdout, stderr } = await execFileAsync(
      process.execPath,
      ["--input-type=module", "--eval", script],
      { cwd, env, maxBuffer: 1024 * 1024 }
    );
    assert.equal(stdout, "");
    assert.equal(stderr, "");
  } catch (error) {
    assert.fail(`Isolated discovery failed: ${String(error)}`);
  }
  return readFile(log, "utf8");
}

async function selectiveFixture(t: TestContext) {
  const result = await fixture(t);
  const { root } = result;
  await write(
    root,
    "turbo.json",
    JSON.stringify({
      futureFlags: {
        experimentalCargoWorkspaces: true,
        experimentalPythonWorkspaces: true
      },
      tasks: {}
    })
  );
  await write(
    root,
    "Cargo.toml",
    '[workspace]\nmembers = ["crates/*"]\nresolver = "2"\n[workspace.metadata]\nname = "rust-aggregate"\n[workspace.dependencies]\nrenamed = { package = "core", path = "crates/core" }\n'
  );
  await write(
    root,
    "crates/app/Cargo.toml",
    '[package]\nname = "rust-app"\nversion = "0.1.0"\n[target.\'cfg(windows)\'.build-dependencies]\nrenamed = { workspace = true, optional = true }\n'
  );
  await write(
    root,
    "python/shared/pyproject.toml",
    '[project]\nname = "Py_Core"\nversion = "0.1.0"\n'
  );
  await write(
    root,
    "python/api/pyproject.toml",
    '[project]\nname = "py-api"\nversion = "0.1.0"\n[project.optional-dependencies]\nfeature = ["Py.Core>=0.1; sys_platform == \'win32\'"]\n[tool.uv.sources]\nPy_Core = [{ workspace = true, marker = "sys_platform == \'win32\'" }, { index = "pypi", marker = "sys_platform != \'win32\'" }]\n'
  );
  return result;
}

describe("StaticWorkspace", () => {
  it("selectively propagates Cargo and uv dependencies without language tools", async (t) => {
    const { dir, root } = await selectiveFixture(t);
    const invoked = await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      assert.equal(workspace.dependencyGraphComplete, false);
      assert.equal(workspace.affectednessComplete, true);
      assert.deepEqual(workspace.unloadedToolchains, ["python", "rust"]);
      for (const [file, expected] of [
        ["apps/web/deleted.js", ["web"]],
        ["crates/core/src/deleted.rs", ["core", "rust-app"]],
        ["crates/app/src/deleted.rs", ["rust-app"]],
        ["python/shared/deleted.py", ["py-api", "py-core"]],
        ["python/api/deleted.py", ["py-api"]],
        ["Cargo.lock", ["core", "rust-app"]],
        ["uv.lock", ["py-api", "py-core"]],
      ]) {
        const candidates = await workspace.affectedCandidates([file]);
        assert.equal(candidates.conservative, false, file);
        assert.deepEqual(candidates.packages.map(p => p.name).sort(), expected, file);
      }
      for (const file of ["Cargo.toml", "pyproject.toml", "crates/removed/Cargo.toml", "python/removed/pyproject.toml"]) {
        assert.deepEqual(await workspace.affectedCandidates([file]), { packages: await workspace.findPackages(), conservative: true });
      }
      assert.deepEqual(await workspace.affectedCandidates([]), { packages: [], conservative: false });
    `,
      process.platform === "win32" ? "empty" : "trap"
    );
    assert.equal(invoked, "");
  });

  it("propagates across co-located native and JavaScript packages", async (t) => {
    const { dir, root } = await selectiveFixture(t);
    await write(
      root,
      "crates/app/package.json",
      JSON.stringify({ name: "rust-wrapper", version: "1.0.0" })
    );
    await write(
      root,
      "apps/web/package.json",
      JSON.stringify({
        name: "web",
        version: "1.0.0",
        dependencies: { "rust-wrapper": "*" }
      })
    );
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      const result = await workspace.affectedCandidates(["crates/core/src/changed.rs"]);
      assert.equal(result.conservative, false);
      assert.deepEqual(result.packages.map(p => p.name).sort(), ["core", "rust-app", "rust-wrapper", "web"]);
    `
    );
  });

  it("retains consumers when a co-located native manifest is removed", async (t) => {
    const { dir, root } = await selectiveFixture(t);
    await write(
      root,
      "crates/app/package.json",
      JSON.stringify({ name: "rust-wrapper", version: "1.0.0" })
    );
    await write(
      root,
      "apps/web/package.json",
      JSON.stringify({ name: "web", dependencies: { "rust-wrapper": "*" } })
    );
    await rm(path.join(root, "crates/app/Cargo.toml"));
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      const result = await workspace.affectedCandidates(["crates/app/Cargo.toml"]);
      assert.equal(result.conservative, true);
      assert.deepEqual(result.packages, await workspace.findPackages());
      assert.ok(result.packages.some(p => p.name === "web"));
    `
    );
  });

  it("matches global inputs inside packages without invalidating unrelated source edits", async (t) => {
    const { dir, root } = await selectiveFixture(t);
    await write(
      root,
      "turbo.json",
      JSON.stringify({
        futureFlags: {
          experimentalCargoWorkspaces: true,
          experimentalPythonWorkspaces: true
        },
        globalDependencies: ["python/shared/**"],
        tasks: {}
      })
    );
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      assert.equal(workspace.affectednessComplete, true);
      assert.deepEqual((await workspace.affectedCandidates(["apps/web/a.js"])).packages.map(p => p.name), ["web"]);
      assert.deepEqual(await workspace.affectedCandidates(["python/shared/deleted.py"]), { packages: await workspace.findPackages(), conservative: true });
    `
    );
  });

  it("resolves uv paths relative to their declaration owner and honors overrides", async (t) => {
    const { dir, root } = await selectiveFixture(t);
    await write(
      root,
      "pyproject.toml",
      '[tool.turbo]\nname = "python-aggregate"\n[tool.uv.workspace]\nmembers = ["python/*"]\n[tool.uv.sources]\nPy_Core = { path = "python/shared", editable = true }\n'
    );
    await write(
      root,
      "python/api/pyproject.toml",
      '[project]\nname = "py-api"\nversion = "0.1.0"\n[dependency-groups]\ndev = ["Py.Core"]\n'
    );
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      assert.equal(workspace.affectednessComplete, true);
      assert.deepEqual((await workspace.affectedCandidates(["python/shared/a.py"])).packages.map(p => p.name).sort(), ["py-api", "py-core"]);
    `
    );
    await write(
      root,
      "python/api/pyproject.toml",
      '[project]\nname = "py-api"\nversion = "0.1.0"\ndependencies = ["Py.Core"]\n[tool.uv.sources]\nPy_Core = { index = "pypi" }\n'
    );
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      assert.equal(workspace.affectednessComplete, true);
      assert.deepEqual((await workspace.affectedCandidates(["python/shared/a.py"])).packages.map(p => p.name), ["py-core"]);
    `
    );
  });

  it("preserves cyclic Cargo development inputs without task-ordering claims", async (t) => {
    const { dir, root } = await selectiveFixture(t);
    await write(
      root,
      "crates/core/Cargo.toml",
      '[package]\nname = "core"\nversion = "0.1.0"\n[dev-dependencies]\napp = { package = "rust-app", path = "../app" }\n'
    );
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      assert.equal(workspace.affectednessComplete, true);
      assert.deepEqual((await workspace.affectedCandidates(["crates/app/a.rs"])).packages.map(p => p.name).sort(), ["core", "rust-app"]);
    `
    );
  });

  for (const [file, contents] of [
    [
      "python/api/pyproject.toml",
      '[project]\nname = "py-api"\ndynamic = ["dependencies"]\n'
    ],
    [".cargo/config.toml", 'paths = ["vendor/override"]\n'],
    ["crates/app/.cargo/config.toml", 'paths = ["vendor/override"]\n'],
    ["uv.toml", 'override-dependencies = ["Py_Core @ file:///unresolved"]\n'],
    [
      "python/api/pyproject.toml",
      '[project]\nname = "py-api"\ndependencies = ["external"]\n[tool.uv.sources]\nexternal = { path = "../../outside-workspace" }\n'
    ]
  ]) {
    it(`reports unresolved static dependencies for ${file}: ${contents}`, async (t) => {
      const { dir, root } = await selectiveFixture(t);
      await write(root, file!, contents!);
      await runIsolated(
        dir,
        root,
        `
        const workspace = await StaticWorkspace.find(root);
        assert.equal(workspace.affectednessComplete, false);
        const result = await workspace.affectedCandidates(["apps/web/a.js"]);
        assert.equal(result.conservative, true);
        assert.deepEqual(result.packages, await workspace.findPackages());
      `
      );
    });
  }
  for (const mode of ["absent", "empty", "trap"] as const) {
    it(
      `inventories all four toolchains with ${mode} PATH`,
      {
        skip: mode === "trap" && process.platform === "win32"
      },
      async (t) => {
        const { dir, root } = await fixture(t);
        const invocations = await runIsolated(
          dir,
          root,
          `
        const workspace = await StaticWorkspace.find(root);
        assert.ok(workspace instanceof StaticWorkspace);
        assert.equal(workspace.absolutePath, root);
        assert.equal(workspace.dependencyGraphComplete, false);
        assert.deepEqual(workspace.unloadedToolchains, ["go", "python", "rust"]);
        const packages = await workspace.findPackages();
        assert.deepEqual(packages, mixedPackages);
        for (const item of packages) assert.equal(Object.getPrototypeOf(item), Object.prototype);
        const roots = workspace.workspaceRoots();
        assert.deepEqual(roots, [
          { toolchain: "go", kind: "go", relativePath: "" },
          { toolchain: "javascript", kind: "npm", relativePath: "" },
          { toolchain: "python", kind: "uv", relativePath: "" },
          { toolchain: "rust", kind: "cargo", relativePath: "" }
        ]);
        for (const item of roots) assert.equal(Object.getPrototypeOf(item), Object.prototype);
        for (const files of [
          ["apps/web/deleted.js"], ["crates/core/missing.rs"],
          ["python/api/deleted.py"], ["go/service/deleted.go"],
          ["unowned/deleted.txt"], ["packages/base/a.js", "packages/base/a.js"],
          ...["package.json", "package-lock.json", "turbo.json", "Cargo.toml", "Cargo.lock",
              "pyproject.toml", "uv.lock", "go.work", "go.work.sum", "go/service/go.mod",
              "go/service/go.sum"].map(file => [file])
        ]) {
          assert.deepEqual(await workspace.affectedCandidates(files), {
            packages: mixedPackages, conservative: true
          }, JSON.stringify(files));
        }
        assert.deepEqual(await workspace.affectedCandidates([]), { packages: [], conservative: false });
        assert.deepEqual(await workspace.findPackages(), packages);
        assert.equal(workspace.dependencyGraphComplete, false);
        assert.deepEqual(workspace.unloadedToolchains, ["go", "python", "rust"]);
        for (const method of ["findPackagesWithGraph", "findPackageByPath", "affectedPackages",
                              "lockfilePackages", "packagesFromLockfile", "tasks"]) {
          assert.equal(workspace[method], undefined, method);
        }
      `,
          mode
        );
        assert.equal(
          invocations,
          "",
          "static discovery must never invoke a language tool"
        );
      }
    );
  }

  it("preserves co-located JavaScript and Rust packages in deterministic order", async (t) => {
    const { dir, root } = await fixture(t);
    await write(
      root,
      "crates/core/package.json",
      JSON.stringify({ name: "zzz-js", version: "1.0.0" })
    );
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      const expected = [...mixedPackages];
      expected.splice(1, 0, pkg("zzz-js", "crates/core"));
      assert.deepEqual(await workspace.findPackages(), expected);
      assert.deepEqual(await workspace.affectedCandidates(["crates/core/deleted.txt"]), {
        packages: expected, conservative: true
      });
    `
    );
  });

  it("maps JavaScript changes through the transitive input-dependent closure", async (t) => {
    const { dir, root } = await fixture(t, false);
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      assert.equal(workspace.dependencyGraphComplete, true);
      assert.deepEqual(workspace.unloadedToolchains, []);
      assert.deepEqual(await workspace.findPackages(), jsPackages);
      for (const [files, packages] of [
        [[], []],
        [["apps/web/deleted.js"], [jsPackages[0]]],
        [["packages/base/deleted.js"], jsPackages.slice(0, 3)],
        [["packages/middle/deleted.js"], [jsPackages[0], jsPackages[2]]],
        [["packages/unrelated/deleted.js"], [jsPackages[3]]],
        [["packages/base/a.js", "packages/base/a.js", "apps/web/b.js"], jsPackages.slice(0, 3)]
      ]) {
        assert.deepEqual(await workspace.affectedCandidates(files), { packages, conservative: false });
      }
      for (const file of ["package.json", "package-lock.json", "turbo.json", "unowned/deleted.txt"]) {
        assert.deepEqual(await workspace.affectedCandidates([file]), {
          packages: jsPackages, conservative: false
        }, file);
      }
      for (const file of ["apps/web/package.json", "packages/deleted/package.json", "apps/web/nested/yarn.lock"]) {
        assert.deepEqual(await workspace.affectedCandidates([file]), {
          packages: jsPackages, conservative: true
        }, file);
      }
    `
    );
  });

  it("suppresses native manifests and unloaded owners when their flags are off", async (t) => {
    const { dir, root } = await fixture(t);
    await write(root, "turbo.json", JSON.stringify({ tasks: {} }));
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      assert.equal(workspace.dependencyGraphComplete, true);
      assert.deepEqual(workspace.unloadedToolchains, []);
      assert.deepEqual(await workspace.findPackages(), jsPackages);
      assert.deepEqual(workspace.workspaceRoots(), [{ toolchain: "javascript", kind: "npm", relativePath: "" }]);
      assert.deepEqual(await workspace.affectedCandidates(["apps/web/deleted.js"]), {
        packages: [jsPackages[0]], conservative: false
      });
    `
    );
  });

  for (const config of [
    { globalDependencies: ["config/**"], tasks: {} },
    {
      futureFlags: { globalConfiguration: true },
      global: { inputs: ["config/**"] },
      tasks: {}
    }
  ]) {
    it(`falls back to all JavaScript packages for configured ${"globalDependencies" in config ? "globalDependencies" : "global.inputs"}`, async (t) => {
      const { dir, root } = await fixture(t, false);
      await write(root, "turbo.json", JSON.stringify(config));
      await runIsolated(
        dir,
        root,
        `
        const workspace = await StaticWorkspace.find(root);
        assert.equal(workspace.dependencyGraphComplete, true);
        assert.deepEqual(workspace.unloadedToolchains, []);
        assert.deepEqual(await workspace.affectedCandidates(["config/shared.json"]), { packages: jsPackages, conservative: true });
        assert.deepEqual(await workspace.affectedCandidates(["apps/web/deleted.js"]), { packages: [jsPackages[0]], conservative: false });
        assert.deepEqual(await workspace.affectedCandidates([]), { packages: [], conservative: false });
      `
      );
    });
  }

  for (const mixed of [false, true]) {
    it(`rejects absolute and escaping change paths before ${mixed ? "conservative fallback" : "JavaScript mapping"}`, async (t) => {
      const { dir, root } = await fixture(t, mixed);
      await runIsolated(
        dir,
        root,
        `
        const workspace = await StaticWorkspace.find(root);
        for (const file of [path.join(root, "apps/web/a.js"), path.resolve(root, "../outside.js"),
                            "../outside.js", "apps/../../outside.js"]) {
          await assert.rejects(() => workspace.affectedCandidates([file]), file);
          await assert.rejects(() => workspace.affectedCandidates(["apps/web/a.js", file]), file);
        }
        assert.deepEqual(await workspace.affectedCandidates([]), { packages: [], conservative: false });
      `
      );
    });
  }

  for (const [file, contents] of [
    ["apps/web/package.json", "{ invalid json"],
    ["crates/core/Cargo.toml", '[package\nname = "broken"'],
    ["python/api/pyproject.toml", '[project\nname = "broken"'],
    ["go/service/go.mod", "go 1.22.0\n"]
  ]) {
    it(`rejects malformed ${file} rather than silently omitting a package`, async (t) => {
      const { dir, root } = await fixture(t);
      await write(root, file!, contents!);
      await runIsolated(
        dir,
        root,
        `
        await assert.rejects(async () => {
          const workspace = await StaticWorkspace.find(root);
          await workspace.findPackages();
        });
      `
      );
    });
  }

  it("uses existing JavaScript root and lockfile-based package-manager inference", async (t) => {
    const { dir, root } = await fixture(t, false);
    await rm(root, { recursive: true });
    await cp(path.resolve(__dirname, "fixtures/npm-monorepo-no-pm"), root, {
      recursive: true
    });
    await runIsolated(
      dir,
      root,
      `
      for (const workspace of [await StaticWorkspace.find(), await StaticWorkspace.find("."),
                               await StaticWorkspace.find(root)]) {
        assert.equal(workspace.absolutePath, root);
        assert.equal(workspace.dependencyGraphComplete, true);
        assert.deepEqual(await workspace.findPackages(), [pkg("app-a", "apps/app"), pkg("ui", "packages/ui")]);
        assert.deepEqual(workspace.workspaceRoots(), [{ toolchain: "javascript", kind: "npm", relativePath: "" }]);
      }
    `,
      "empty",
      root
    );
  });

  it("infers a declared JavaScript workspace from a nested directory", async (t) => {
    const { dir, root } = await fixture(t, false);
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find();
      const existing = await Workspace.find(undefined, { skipPackageGraph: true });
      assert.equal(workspace.absolutePath, root);
      assert.equal(workspace.absolutePath, existing.absolutePath);
      assert.deepEqual(await workspace.findPackages(), jsPackages);
    `,
      "empty",
      path.join(root, "apps/web")
    );
  });

  it("ignores absent native workspaces even when their flags are enabled", async (t) => {
    const { dir, root } = await fixture(t, false);
    await write(
      root,
      "turbo.json",
      JSON.stringify({ futureFlags: nativeFlags, tasks: {} })
    );
    await runIsolated(
      dir,
      root,
      `
      const workspace = await StaticWorkspace.find(root);
      assert.equal(workspace.dependencyGraphComplete, true);
      assert.deepEqual(workspace.unloadedToolchains, []);
      assert.deepEqual(await workspace.findPackages(), jsPackages);
      assert.deepEqual(await workspace.affectedCandidates(["packages/base/deleted.js"]), {
        packages: jsPackages.slice(0, 3), conservative: false
      });
    `
    );
  });

  it("does not infer a native-only root without the JavaScript workspace context", async (t) => {
    const { dir, root } = await fixture(t);
    await rm(path.join(root, "package.json"));
    await rm(path.join(root, "package-lock.json"));
    await runIsolated(
      dir,
      root,
      `await assert.rejects(() => StaticWorkspace.find(root));`
    );
  });

  it(
    "leaves default full discovery authoritative and skipPackageGraph lockfile-only",
    {
      skip: process.platform === "win32"
    },
    async (t) => {
      const { dir, root } = await fixture(t);
      await write(
        root,
        "turbo.json",
        JSON.stringify({
          futureFlags: { experimentalCargoWorkspaces: true },
          tasks: {}
        })
      );
      const invocations = await runIsolated(
        dir,
        root,
        `
      const skipped = await Workspace.find(root, { skipPackageGraph: true });
      assert.equal(skipped.absolutePath, root);
      await skipped.lockfilePackages();
      for (const action of [() => skipped.findPackages(), () => skipped.findPackagesWithGraph(),
                            () => skipped.affectedPackages(["apps/web/a.js"])]) {
        await assert.rejects(action);
      }
      assert.equal(readFileSync(log, "utf8"), "");
      // This is deliberately separate from all static-discovery setup. Full
      // discovery must attempt authoritative Cargo metadata and surface failure.
      await assert.rejects(() => Workspace.find(root), /cargo/i);
      assert.match(readFileSync(log, "utf8"), /^cargo$/m);
    `,
        "trap"
      );
      assert.match(invocations, /^cargo$/m);
    }
  );
});
