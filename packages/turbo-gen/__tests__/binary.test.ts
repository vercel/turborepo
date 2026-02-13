import { describe, it, expect, beforeAll, afterAll } from "@jest/globals";
import { execSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const BINARY = path.resolve(__dirname, "..", "dist", "turbo-gen");
const SHIM = path.resolve(__dirname, "..", "bin", "turbo-gen");
const binaryExists = fs.existsSync(BINARY);

function bin(args: string[], cwd?: string): string {
  const escaped = args.map((a) => `'${a.replace(/'/g, "'\\''")}'`).join(" ");
  return execSync(`'${BINARY}' ${escaped}`, {
    cwd: cwd ?? os.tmpdir(),
    timeout: 15000,
    encoding: "utf-8",
    stdio: ["pipe", "pipe", "pipe"]
  });
}

function shim(args: string[], cwd?: string): string {
  const escaped = args.map((a) => `'${a.replace(/'/g, "'\\''")}'`).join(" ");
  return execSync(`'${SHIM}' ${escaped}`, {
    cwd: cwd ?? os.tmpdir(),
    timeout: 15000,
    encoding: "utf-8",
    env: { ...process.env, TURBO_GEN_BINARY_PATH: BINARY },
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

const TS_CONFIG = (name: string) => `
export default function generator(plop: any): void {
  plop.setGenerator("${name}", {
    description: "${name}",
    prompts: [],
    actions: [{ type: "add", path: "out/${name}.md", template: "# ${name}" }]
  });
}
`;

const JS_CJS_CONFIG = (name: string) => `
module.exports = function generator(plop) {
  plop.setGenerator("${name}", {
    description: "${name}",
    prompts: [],
    actions: [{ type: "add", path: "out/${name}.md", template: "# ${name}" }]
  });
};
`;

// Skip the entire suite if the binary hasn't been built.
// Unit tests (raw.test.ts) always run; these are integration tests
// that require `pnpm --filter @turbo/gen run build` first.
const describeIfBinary = binaryExists ? describe : describe.skip;

describeIfBinary("compiled binary", () => {
  let tmpDir: string;

  beforeAll(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "turbo-gen-test-"));
  });

  afterAll(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  describe("cli basics", () => {
    it("--version returns the package version", () => {
      const out = bin(["--version"]);
      expect(out.trim()).toMatch(/^\d+\.\d+\.\d+/);
    });

    it("--help lists commands", () => {
      const out = bin(["--help"]);
      expect(out).toContain("Extend your Turborepo");
      expect(out).toContain("run");
      expect(out).toContain("workspace");
    });
  });

  describe("shim", () => {
    it("resolves via TURBO_GEN_BINARY_PATH", () => {
      const out = shim(["--version"]);
      expect(out.trim()).toMatch(/^\d+\.\d+\.\d+/);
    });

    it("exits with clear error on bad TURBO_GEN_BINARY_PATH", () => {
      expect(() =>
        execSync(`'${SHIM}' --version`, {
          encoding: "utf-8",
          env: {
            ...process.env,
            TURBO_GEN_BINARY_PATH: "/nonexistent/turbo-gen"
          },
          stdio: ["pipe", "pipe", "pipe"]
        })
      ).toThrow(/TURBO_GEN_BINARY_PATH/);
    });
  });

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
    }
  ])(
    "$label",
    ({ label, type, configFile, configContent, generatorPkgType }) => {
      let projectDir: string;
      // Derive a unique name from the full label to avoid collisions
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
        // Clean output from any prior run
        fs.rmSync(path.join(projectDir, "out"), {
          recursive: true,
          force: true
        });

        bin(
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
      bin(
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

    it("raw run via shim produces identical output", () => {
      fs.rmSync(path.join(projectDir, "out"), {
        recursive: true,
        force: true
      });

      shim(
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
        bin(
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
