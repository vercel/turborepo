import { describe, it, expect, beforeAll, afterAll } from "@jest/globals";
import { execSync, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const CLI = path.resolve(__dirname, "..", "dist", "cli.js");
const cliExists = fs.existsSync(CLI);

function run(args: string[], cwd?: string): string {
  const escaped = args.map((a) => `'${a.replace(/'/g, "'\\''")}'`).join(" ");
  return execSync(`node '${CLI}' ${escaped}`, {
    cwd: cwd ?? os.tmpdir(),
    timeout: 30000,
    encoding: "utf-8",
    stdio: ["pipe", "pipe", "pipe"]
  });
}

function createProject(
  root: string,
  opts: {
    type?: "commonjs" | "module";
    configFile: string;
    configContent: string;
    generatorPkgType?: string;
  }
) {
  const genDir = path.join(root, "turbo", "generators");
  fs.mkdirSync(genDir, { recursive: true });

  const pkg: Record<string, unknown> = { name: "test", version: "1.0.0" };
  if (opts.type) pkg.type = opts.type;
  fs.writeFileSync(path.join(root, "package.json"), JSON.stringify(pkg));
  fs.writeFileSync(path.join(root, "turbo.json"), "{}");
  fs.writeFileSync(
    path.join(root, "package-lock.json"),
    '{"lockfileVersion":3}'
  );

  fs.writeFileSync(path.join(genDir, opts.configFile), opts.configContent);

  if (opts.generatorPkgType) {
    fs.writeFileSync(
      path.join(genDir, "package.json"),
      JSON.stringify({ type: opts.generatorPkgType })
    );
  }
}

// TypeScript config (works for .ts and .mts)
const TS_CONFIG = (name: string) => `
export default function generator(plop: any): void {
  plop.setGenerator("${name}", {
    description: "${name}",
    prompts: [],
    actions: [{ type: "add", path: "out/${name}.md", template: "# ${name}" }]
  });
}
`;

// CJS config (works for .js without "type":"module" and .cjs)
const JS_CJS_CONFIG = (name: string) => `
module.exports = function generator(plop) {
  plop.setGenerator("${name}", {
    description: "${name}",
    prompts: [],
    actions: [{ type: "add", path: "out/${name}.md", template: "# ${name}" }]
  });
};
`;

// ESM config (works for .mjs and .js with "type":"module")
const JS_ESM_CONFIG = (name: string) => `
export default function generator(plop) {
  plop.setGenerator("${name}", {
    description: "${name}",
    prompts: [],
    actions: [{ type: "add", path: "out/${name}.md", template: "# ${name}" }]
  });
}
`;

// Config that imports an external npm package from the project's node_modules.
const TS_CONFIG_WITH_EXTERNAL_IMPORT = (name: string) => `
import { helper } from "test-external-pkg";

export default function generator(plop: any): void {
  plop.setGenerator("${name}", {
    description: helper("${name}"),
    prompts: [],
    actions: [{ type: "add", path: "out/${name}.md", template: "# ${name}" }]
  });
}
`;

// Config that imports @inquirer/prompts, which is a dependency of @turbo/gen
// but NOT installed in the user's node_modules.
// Regression test for https://github.com/vercel/turborepo/issues/11855
const TS_CONFIG_WITH_CLI_MODULE_IMPORT = (name: string) => `
import { Separator } from "@inquirer/prompts";

const sep = new Separator();

export default function generator(plop: any): void {
  plop.setGenerator("${name}", {
    description: "${name}",
    prompts: [],
    actions: [{ type: "add", path: "out/${name}.md", template: "# ${name}" }]
  });
}
`;

// Config that imports BOTH an external npm package AND @inquirer/prompts
// (which is NOT installed in the user's project). This is the realistic case:
// a user installs a helper like slugify and also uses @inquirer/prompts
// features in the same config file.
const TS_CONFIG_WITH_EXTERNAL_AND_CLI_MODULE = (name: string) => `
import { helper } from "test-external-pkg";
import { Separator } from "@inquirer/prompts";

const sep = new Separator();

export default function generator(plop: any): void {
  plop.setGenerator("${name}", {
    description: helper("${name}"),
    prompts: [],
    actions: [{ type: "add", path: "out/${name}.md", template: "# ${name}" }]
  });
}
`;

// Config that delegates to a sub-generator file which imports an external
// npm package. The package is installed at the project root's node_modules/
// (NOT next to the sub-generator). This is the exact scenario from #11882:
// turbo/generators/config.ts -> turbo/generators/sub-gen/generator.ts -> slugify
const TS_CONFIG_WITH_SUB_GENERATOR = (name: string) => `
import { subGenerator } from "./sub-gen/generator";

export default function generator(plop: any): void {
  plop.setGenerator("${name}", subGenerator);
}
`;

const TS_SUB_GENERATOR = (name: string) => `
import { helper } from "test-external-pkg";

export const subGenerator = {
  description: helper("${name}"),
  prompts: [],
  actions: [{ type: "add", path: "out/${name}.md", template: "# ${name}" }]
};
`;

/**
 * Creates a fake npm package in the project's node_modules so that
 * config files can import it without running a real package manager.
 */
function createFakePackage(
  projectRoot: string,
  packageName: string,
  code: string
) {
  const pkgDir = path.join(projectRoot, "node_modules", packageName);
  fs.mkdirSync(pkgDir, { recursive: true });
  fs.writeFileSync(
    path.join(pkgDir, "package.json"),
    JSON.stringify({ name: packageName, version: "1.0.0", main: "index.js" })
  );
  fs.writeFileSync(path.join(pkgDir, "index.js"), code);
}

// Skip the entire suite if the CLI hasn't been built.
// Unit tests (raw.test.ts) always run; these are integration tests
// that require `pnpm --filter @turbo/gen run build` first.
const describeIfBuilt = cliExists ? describe : describe.skip;

describeIfBuilt("@turbo/gen CLI", () => {
  let tmpDir: string;

  beforeAll(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "turbo-gen-test-"));
  });

  afterAll(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  describe("cli basics", () => {
    it("--version returns the package version", () => {
      const out = run(["--version"]);
      expect(out.trim()).toMatch(/^\d+\.\d+\.\d+/);
    });

    it("--help lists commands", () => {
      const out = run(["--help"]);
      expect(out).toContain("Extend your Turborepo");
      expect(out).toContain("run");
      expect(out).toContain("workspace");
    });
  });

  // ESM/CJS config loading matrix — covers every realistic combination of
  // project "type" field, config file extension, and module syntax.
  describe.each([
    {
      label: 'CJS project + .ts config ("type":"commonjs")',
      type: "commonjs" as const,
      configFile: "config.ts",
      configContent: TS_CONFIG,
      generatorPkgType: "commonjs"
    },
    {
      label: 'ESM project + .ts config ("type":"module", gen has "commonjs")',
      type: "module" as const,
      configFile: "config.ts",
      configContent: TS_CONFIG,
      generatorPkgType: "commonjs"
    },
    {
      label: "ESM project + .ts config (no generator package.json)",
      type: "module" as const,
      configFile: "config.ts",
      configContent: TS_CONFIG,
      generatorPkgType: undefined
    },
    {
      label: "No type field + .js CJS config",
      type: undefined,
      configFile: "config.js",
      configContent: JS_CJS_CONFIG,
      generatorPkgType: undefined
    },
    {
      label: "CJS project + .cjs config",
      type: "commonjs" as const,
      configFile: "config.cjs",
      configContent: JS_CJS_CONFIG,
      generatorPkgType: undefined
    },
    {
      label: 'ESM project + .js CJS config (gen dir "commonjs")',
      type: "module" as const,
      configFile: "config.js",
      configContent: JS_CJS_CONFIG,
      generatorPkgType: "commonjs"
    },
    {
      label: "ESM project + .cjs config",
      type: "module" as const,
      configFile: "config.cjs",
      configContent: JS_CJS_CONFIG,
      generatorPkgType: undefined
    },
    // .mjs configs (ESM syntax)
    {
      label: 'ESM project + .mjs config ("type":"module")',
      type: "module" as const,
      configFile: "config.mjs",
      configContent: JS_ESM_CONFIG,
      generatorPkgType: undefined
    },
    {
      label: "No type field + .mjs config",
      type: undefined,
      configFile: "config.mjs",
      configContent: JS_ESM_CONFIG,
      generatorPkgType: undefined
    },
    // .mts configs (TypeScript + ESM syntax)
    {
      label: 'CJS project + .mts config ("type":"commonjs")',
      type: "commonjs" as const,
      configFile: "config.mts",
      configContent: TS_CONFIG,
      generatorPkgType: undefined
    },
    {
      label: 'ESM project + .mts config ("type":"module")',
      type: "module" as const,
      configFile: "config.mts",
      configContent: TS_CONFIG,
      generatorPkgType: undefined
    },
    // .js with ESM syntax in "type":"module" project
    {
      label:
        'ESM project + .js ESM config ("type":"module", no gen dir override)',
      type: "module" as const,
      configFile: "config.js",
      configContent: JS_ESM_CONFIG,
      generatorPkgType: undefined
    }
  ])(
    "$label",
    ({ label, type, configFile, configContent, generatorPkgType }) => {
      let projectDir: string;
      const genName = label
        .replace(/[^a-zA-Z0-9]+/g, "-")
        .toLowerCase()
        .substring(0, 40);

      beforeAll(() => {
        projectDir = path.join(tmpDir, genName);
        fs.mkdirSync(projectDir, { recursive: true });
        createProject(projectDir, {
          type,
          configFile,
          configContent: configContent(genName),
          generatorPkgType
        });
      });

      it("loads the generator and runs actions", () => {
        fs.rmSync(path.join(projectDir, "out"), {
          recursive: true,
          force: true
        });

        run(
          [
            "raw",
            "run",
            "--json",
            JSON.stringify({ root: projectDir, generator_name: genName })
          ],
          projectDir
        );

        const outFile = path.join(projectDir, "out", `${genName}.md`);
        expect(fs.existsSync(outFile)).toBe(true);
        expect(fs.readFileSync(outFile, "utf-8")).toContain(`# ${genName}`);
      });
    }
  );

  // Regression test for https://github.com/vercel/turborepo/issues/11855
  describe("config importing external npm packages", () => {
    let projectDir: string;
    const genName = "ext-import-test";

    beforeAll(() => {
      projectDir = path.join(tmpDir, "external-import");
      fs.mkdirSync(projectDir, { recursive: true });
      createProject(projectDir, {
        type: "commonjs",
        configFile: "config.ts",
        configContent: TS_CONFIG_WITH_EXTERNAL_IMPORT(genName),
        generatorPkgType: "commonjs"
      });
      createFakePackage(
        projectDir,
        "test-external-pkg",
        'module.exports.helper = function(name) { return name + " via external"; };'
      );
    });

    it("resolves npm packages from the project node_modules", () => {
      fs.rmSync(path.join(projectDir, "out"), {
        recursive: true,
        force: true
      });

      run(
        [
          "raw",
          "run",
          "--json",
          JSON.stringify({
            root: projectDir,
            generator_name: genName
          })
        ],
        projectDir
      );

      const outFile = path.join(projectDir, "out", `${genName}.md`);
      expect(fs.existsSync(outFile)).toBe(true);
      expect(fs.readFileSync(outFile, "utf-8")).toContain(`# ${genName}`);
    });
  });

  // Regression test for https://github.com/vercel/turborepo/issues/11855
  // @inquirer/prompts is a dependency of @turbo/gen but NOT in the test
  // project's node_modules — the CLI's resolve fallback must find it.
  describe("config importing CLI-provided modules (@inquirer/prompts)", () => {
    let projectDir: string;
    const genName = "cli-module-test";

    beforeAll(() => {
      projectDir = path.join(tmpDir, "cli-module-import");
      fs.mkdirSync(projectDir, { recursive: true });
      createProject(projectDir, {
        type: "commonjs",
        configFile: "config.ts",
        configContent: TS_CONFIG_WITH_CLI_MODULE_IMPORT(genName),
        generatorPkgType: "commonjs"
      });
      // Intentionally NO node_modules/@inquirer/prompts on disk
    });

    it("resolves @inquirer/prompts from @turbo/gen dependencies", () => {
      fs.rmSync(path.join(projectDir, "out"), {
        recursive: true,
        force: true
      });

      run(
        [
          "raw",
          "run",
          "--json",
          JSON.stringify({
            root: projectDir,
            generator_name: genName
          })
        ],
        projectDir
      );

      const outFile = path.join(projectDir, "out", `${genName}.md`);
      expect(fs.existsSync(outFile)).toBe(true);
      expect(fs.readFileSync(outFile, "utf-8")).toContain(`# ${genName}`);
    });
  });

  // Combined test: external npm package + CLI-provided module in the same config.
  // This is the most realistic scenario — a user has a helper dep installed AND
  // uses @inquirer/prompts features, all in one config file.
  describe("config importing external package AND @inquirer/prompts together", () => {
    let projectDir: string;
    const genName = "combined-import-test";

    beforeAll(() => {
      projectDir = path.join(tmpDir, "combined-import");
      fs.mkdirSync(projectDir, { recursive: true });
      createProject(projectDir, {
        type: "commonjs",
        configFile: "config.ts",
        configContent: TS_CONFIG_WITH_EXTERNAL_AND_CLI_MODULE(genName),
        generatorPkgType: "commonjs"
      });
      createFakePackage(
        projectDir,
        "test-external-pkg",
        'module.exports.helper = function(name) { return name + " via external"; };'
      );
      // Intentionally NO @inquirer/prompts in this project's node_modules
    });

    it("resolves both the npm package and @inquirer/prompts", () => {
      fs.rmSync(path.join(projectDir, "out"), {
        recursive: true,
        force: true
      });

      run(
        [
          "raw",
          "run",
          "--json",
          JSON.stringify({
            root: projectDir,
            generator_name: genName
          })
        ],
        projectDir
      );

      const outFile = path.join(projectDir, "out", `${genName}.md`);
      expect(fs.existsSync(outFile)).toBe(true);
      expect(fs.readFileSync(outFile, "utf-8")).toContain(`# ${genName}`);
    });
  });

  // Regression test for https://github.com/vercel/turborepo/issues/11882
  // The package is in ROOT/node_modules/ but the importing file is in
  // turbo/generators/sub-gen/ — resolution must walk up the directory tree.
  describe("sub-generator importing from ancestor node_modules", () => {
    let projectDir: string;
    const genName = "sub-gen-import-test";

    beforeAll(() => {
      projectDir = path.join(tmpDir, "sub-gen-import");
      fs.mkdirSync(projectDir, { recursive: true });
      createProject(projectDir, {
        type: "commonjs",
        configFile: "config.ts",
        configContent: TS_CONFIG_WITH_SUB_GENERATOR(genName),
        generatorPkgType: "commonjs"
      });
      const subGenDir = path.join(projectDir, "turbo", "generators", "sub-gen");
      fs.mkdirSync(subGenDir, { recursive: true });
      fs.writeFileSync(
        path.join(subGenDir, "generator.ts"),
        TS_SUB_GENERATOR(genName)
      );
      createFakePackage(
        projectDir,
        "test-external-pkg",
        'module.exports.helper = function(name) { return name + " via external"; };'
      );
    });

    it("resolves packages from ancestor node_modules", () => {
      fs.rmSync(path.join(projectDir, "out"), {
        recursive: true,
        force: true
      });

      run(
        [
          "raw",
          "run",
          "--json",
          JSON.stringify({
            root: projectDir,
            generator_name: genName
          })
        ],
        projectDir
      );

      const outFile = path.join(projectDir, "out", `${genName}.md`);
      expect(fs.existsSync(outFile)).toBe(true);
      expect(fs.readFileSync(outFile, "utf-8")).toContain(`# ${genName}`);
    });
  });

  describe("SIGINT handling", () => {
    let projectDir: string;

    beforeAll(() => {
      projectDir = path.join(tmpDir, "sigint-test");
      fs.mkdirSync(projectDir, { recursive: true });
      createProject(projectDir, {
        type: "commonjs",
        configFile: "config.ts",
        configContent: TS_CONFIG("sigint-gen"),
        generatorPkgType: "commonjs"
      });
    });

    it("exits cleanly on SIGINT without ExitPromptError stack trace", (done) => {
      const child = spawn("node", [CLI, "run"], {
        cwd: projectDir,
        stdio: ["pipe", "pipe", "pipe"],
        env: { ...process.env }
      });

      let stderr = "";
      child.stderr.on("data", (data: Buffer) => {
        stderr += data.toString();
      });

      setTimeout(() => child.kill("SIGINT"), 500);

      child.on("close", () => {
        expect(stderr).not.toContain("ExitPromptError");
        expect(stderr).not.toContain("Unexpected error");
        done();
      });
    });
  });

  describe("raw command (Rust CLI handoff)", () => {
    let projectDir: string;

    beforeAll(() => {
      projectDir = path.join(tmpDir, "raw-handoff");
      fs.mkdirSync(projectDir, { recursive: true });
      createProject(projectDir, {
        type: "commonjs",
        configFile: "config.ts",
        configContent: TS_CONFIG("raw-handoff"),
        generatorPkgType: "commonjs"
      });
    });

    it("raw run dispatches to the correct generator", () => {
      run(
        [
          "raw",
          "run",
          "--json",
          JSON.stringify({
            root: projectDir,
            generator_name: "raw-handoff"
          })
        ],
        projectDir
      );

      const outFile = path.join(projectDir, "out", "raw-handoff.md");
      expect(fs.existsSync(outFile)).toBe(true);
    });

    it("raw workspace does not crash with module errors", () => {
      try {
        run(
          [
            "raw",
            "workspace",
            "--json",
            JSON.stringify({ root: projectDir, empty: true })
          ],
          projectDir
        );
      } catch (e: any) {
        const output = (e.stdout ?? "") + (e.stderr ?? "");
        // It may fail asking for interactive input. That's fine.
        // It must NOT fail with ESM/CJS errors.
        expect(output).not.toMatch(/Cannot find module/);
        expect(output).not.toMatch(/ERR_REQUIRE_ESM/);
        expect(output).not.toMatch(/ERR_MODULE_NOT_FOUND/);
        expect(output).not.toMatch(/SyntaxError.*import/);
      }
    });
  });
});
