import { exec as execCb } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { PackageManagerType, TestCase, TestResult } from "../types";

function exec(
  command: string,
  cwd: string,
  env?: Record<string, string>
): Promise<{ exitCode: number; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    execCb(
      command,
      {
        cwd,
        env: { ...process.env, ...env },
        timeout: 10 * 60 * 1000,
        maxBuffer: 10 * 1024 * 1024
      },
      (error, stdout, stderr) => {
        if (error) {
          const exitCode =
            typeof error.code === "number"
              ? error.code
              : ((error as unknown as { status?: number }).status ?? 1);
          resolve({
            exitCode,
            stdout: stdout?.toString() ?? "",
            stderr: stderr?.toString() ?? ""
          });
          return;
        }
        resolve({
          exitCode: 0,
          stdout: stdout?.toString() ?? "",
          stderr: stderr?.toString() ?? ""
        });
      }
    );
  });
}

function copyDirSync(src: string, dest: string): void {
  fs.mkdirSync(dest, { recursive: true });
  for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);
    if (entry.isDirectory()) {
      copyDirSync(srcPath, destPath);
    } else {
      fs.copyFileSync(srcPath, destPath);
    }
  }
}

function lightweightValidationCommand(
  pm: PackageManagerType,
  frozenInstallCommand: string[]
): string {
  switch (pm) {
    case "pnpm": {
      return "pnpm install --frozen-lockfile --lockfile-only";
    }
    case "npm": {
      return "npm install --package-lock-only --ignore-scripts";
    }
    case "yarn": {
      return "yarn install --frozen-lockfile --ignore-scripts";
    }
    case "yarn-berry": {
      return "yarn install --immutable --mode=skip-build";
    }
    case "bun": {
      return frozenInstallCommand.join(" ");
    }
  }
}

// Cached result of setting up a package manager (corepack or bun) once.
interface PmEnvCache {
  pmBinDirs: string[];
}

export class LocalRunner {
  // Tracks which fixtures have been validated (keyed by filepath)
  private validated = new Map<string, string | null>();
  // In-flight validation promises so concurrent tests for the same fixture wait
  private validating = new Map<string, Promise<string | null>>();

  // Cached PM environments keyed by packageManagerVersion (e.g. "pnpm@9.15.0")
  private pmEnvCache = new Map<string, PmEnvCache>();
  private pmEnvInflight = new Map<string, Promise<PmEnvCache>>();

  /**
   * Gets or creates a cached package manager environment. Runs corepack/bun
   * setup once per unique packageManagerVersion, reuses for all subsequent calls.
   */
  private getPmEnv(
    fixture: TestCase["fixture"],
    log: (...args: unknown[]) => void
  ): Promise<PmEnvCache> {
    const key = fixture.packageManagerVersion;

    const cached = this.pmEnvCache.get(key);
    if (cached) return Promise.resolve(cached);

    const inflight = this.pmEnvInflight.get(key);
    if (inflight) return inflight;

    const promise = this.createPmEnv(fixture, log).then((env) => {
      this.pmEnvCache.set(key, env);
      this.pmEnvInflight.delete(key);
      return env;
    });
    this.pmEnvInflight.set(key, promise);
    return promise;
  }

  private async createPmEnv(
    fixture: TestCase["fixture"],
    log: (...args: unknown[]) => void
  ): Promise<PmEnvCache> {
    const envDir = fs.mkdtempSync(path.join(os.tmpdir(), "lockfile-pmenv-"));
    const pmBinDirs: string[] = [];

    const corepackBin = path.join(envDir, "corepack-bin");
    fs.mkdirSync(corepackBin, { recursive: true });
    await exec(`corepack enable --install-directory "${corepackBin}"`, envDir);
    pmBinDirs.push(corepackBin);

    if (fixture.packageManager === "bun") {
      const bunVersion = fixture.packageManagerVersion.replace("bun@", "");
      const bunDir = path.join(envDir, "bun-install");

      log(`[pmenv] Installing bun@${bunVersion} (shared)`);
      const bunInstall = await exec(
        `curl -fsSL https://bun.sh/install | BUN_INSTALL="${bunDir}" bash -s "bun-v${bunVersion}"`,
        envDir,
        { PATH: `${corepackBin}:${process.env.PATH}` }
      );
      if (bunInstall.exitCode !== 0) {
        throw new Error(
          `Failed to install bun@${bunVersion}: ${bunInstall.stderr}`
        );
      }
      pmBinDirs.unshift(`${bunDir}/bin`);
    } else {
      log(`[pmenv] corepack prepare ${fixture.packageManagerVersion} (shared)`);
      const prep = await exec(
        `corepack prepare ${fixture.packageManagerVersion} --activate`,
        envDir,
        {
          PATH: `${corepackBin}:${process.env.PATH}`,
          COREPACK_ENABLE_STRICT: "0"
        }
      );
      if (prep.exitCode !== 0) {
        log(`[pmenv] corepack prepare warning: ${prep.stderr || prep.stdout}`);
      }
    }

    return { pmBinDirs };
  }

  /**
   * Validates a fixture once. Returns null if valid, or an error message.
   * Concurrent calls for the same fixture share a single validation run.
   */
  private validateFixture(
    fixture: TestCase["fixture"],
    turboBinaryPath: string
  ): Promise<string | null> {
    const key = fixture.filepath;

    if (this.validated.has(key)) {
      return Promise.resolve(this.validated.get(key)!);
    }

    if (this.validating.has(key)) {
      return this.validating.get(key)!;
    }

    const promise = this.doValidate(fixture, turboBinaryPath).then((err) => {
      this.validated.set(key, err);
      this.validating.delete(key);
      return err;
    });
    this.validating.set(key, promise);
    return promise;
  }

  private async doValidate(
    fixture: TestCase["fixture"],
    turboBinaryPath: string
  ): Promise<string | null> {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lockfile-validate-"));

    try {
      copyDirSync(fixture.filepath, tmpDir);
      const fullPath = await this.buildPath(
        fixture,
        tmpDir,
        turboBinaryPath,
        console.log
      );

      const validationCmd = lightweightValidationCommand(
        fixture.packageManager,
        fixture.frozenInstallCommand
      );
      console.log(
        `[${fixture.filename}] Validating fixture (${validationCmd})...`
      );
      const result = await exec(validationCmd, tmpDir, {
        PATH: fullPath,
        COREPACK_ENABLE_STRICT: "0"
      });

      if (result.exitCode !== 0) {
        const output = [result.stdout, result.stderr]
          .filter(Boolean)
          .join("\n");
        return (
          `INVALID FIXTURE: frozen install fails on unpruned original (exit ${result.exitCode}).\n` +
          `This means the fixture's package.jsons don't match its lockfile.\n` +
          `Fix the fixture or rebuild it from a real repo.\n\n${output}`
        );
      }

      console.log(`[${fixture.filename}] Fixture validated`);
      return null;
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  }

  /**
   * Builds PATH for a temp dir by combining the cached PM env with a
   * per-tmpdir turbo symlink. No corepack/bun setup work happens here.
   */
  private async buildPath(
    fixture: TestCase["fixture"],
    tmpDir: string,
    turboBinaryPath: string,
    log: (...args: unknown[]) => void
  ): Promise<string> {
    const localBin = path.join(tmpDir, ".bin");
    fs.mkdirSync(localBin, { recursive: true });
    if (turboBinaryPath) {
      fs.symlinkSync(turboBinaryPath, path.join(localBin, "turbo"));
    }

    const pmEnv = await this.getPmEnv(fixture, log);
    return [...pmEnv.pmBinDirs, localBin, process.env.PATH].join(":");
  }

  /**
   * Runs all test cases for a single fixture using one shared temp dir
   * and one git init. Workspace targets are tested sequentially within
   * the shared copy, avoiding redundant copies and git inits.
   */
  async runFixtureGroup(
    testCases: TestCase[],
    turboBinaryPath: string
  ): Promise<TestResult[]> {
    if (testCases.length === 0) return [];

    const fixture = testCases[0].fixture;
    const results: TestResult[] = [];

    // Validate once per fixture
    const validationError = await this.validateFixture(
      fixture,
      turboBinaryPath
    );
    if (validationError) {
      for (const tc of testCases) {
        results.push({
          label: tc.label,
          success: false,
          pruneSuccess: false,
          installSuccess: false,
          error: validationError,
          durationMs: 0
        });
      }
      return results;
    }

    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lockfile-test-"));

    try {
      console.log(`[${fixture.filename}] Copying fixture to ${tmpDir}`);
      copyDirSync(fixture.filepath, tmpDir);

      const fullPath = await this.buildPath(
        fixture,
        tmpDir,
        turboBinaryPath,
        console.log
      );
      const installCmd = fixture.frozenInstallCommand.join(" ");

      // Git init once for all workspace targets
      await exec(
        'git init && git add . && git commit --allow-empty -m "init"',
        tmpDir,
        {
          PATH: fullPath,
          GIT_AUTHOR_NAME: "test",
          GIT_AUTHOR_EMAIL: "test@test.com",
          GIT_COMMITTER_NAME: "test",
          GIT_COMMITTER_EMAIL: "test@test.com"
        }
      );

      for (const tc of testCases) {
        const { targetWorkspace, label, expectedFailure } = tc;
        const quiet = !!expectedFailure;
        const log = quiet ? (..._args: unknown[]) => {} : console.log;
        const startTime = Date.now();
        const result: TestResult = {
          label,
          success: false,
          pruneSuccess: false,
          installSuccess: false,
          durationMs: 0
        };

        try {
          // turbo prune
          log(`[${label}] turbo prune ${targetWorkspace.name}`);
          const pruneResult = await exec(
            `turbo prune ${targetWorkspace.name}`,
            tmpDir,
            { PATH: fullPath }
          );

          result.pruneOutput = [pruneResult.stdout, pruneResult.stderr]
            .filter(Boolean)
            .join("\n");

          if (pruneResult.exitCode !== 0) {
            log(`[${label}] PRUNE FAILED (exit ${pruneResult.exitCode})`);
            result.error = `Prune failed:\n${result.pruneOutput}`;
            result.durationMs = Date.now() - startTime;
            results.push(result);
            continue;
          }

          result.pruneSuccess = true;
          log(`[${label}] Prune succeeded`);

          // Frozen install in pruned output
          const outDir = path.join(tmpDir, "out");
          log(`[${label}] ${installCmd} (in out/)`);

          const installResult = await exec(installCmd, outDir, {
            PATH: fullPath,
            COREPACK_ENABLE_STRICT: "0"
          });

          result.installOutput = [installResult.stdout, installResult.stderr]
            .filter(Boolean)
            .join("\n");

          if (installResult.exitCode !== 0) {
            log(
              `[${label}] FROZEN INSTALL FAILED (exit ${installResult.exitCode})`
            );
            result.error = `Frozen install failed:\n${result.installOutput}`;
            result.durationMs = Date.now() - startTime;
            results.push(result);
            continue;
          }

          result.installSuccess = true;
          result.success = true;
          log(`[${label}] PASSED`);
        } catch (err) {
          result.error = err instanceof Error ? err.message : String(err);
          if (!quiet) console.error(`[${label}] ERROR: ${result.error}`);
        } finally {
          // Clean up out/ for the next workspace target
          const outDir = path.join(tmpDir, "out");
          try {
            fs.rmSync(outDir, { recursive: true, force: true });
          } catch {
            // best-effort
          }
          result.durationMs = Date.now() - startTime;
        }

        results.push(result);
      }
    } finally {
      try {
        fs.rmSync(tmpDir, { recursive: true, force: true });
      } catch {
        // best-effort
      }
    }

    return results;
  }
}
