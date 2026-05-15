import { exec as execCb } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { PackageManagerType, TestCase, TestResult } from "../types";

interface ExecResult {
  exitCode: number;
  stdout: string;
  stderr: string;
}

interface LockfileValidationCommand {
  command: string;
  env?: Record<string, string>;
  acceptsFailure?: (result: ExecResult) => boolean;
  verifyLockfileUnchanged?: boolean;
}

function exec(
  command: string,
  cwd: string,
  env?: Record<string, string>
): Promise<ExecResult> {
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

function combinedOutput(result: ExecResult): string {
  return [result.stdout, result.stderr].filter(Boolean).join("\n");
}

function readLockfile(cwd: string, lockfileName: string): Buffer | null {
  const lockfilePath = path.join(cwd, lockfileName);
  if (!fs.existsSync(lockfilePath)) return null;
  return fs.readFileSync(lockfilePath);
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

function lockfileValidationCommand(
  pm: PackageManagerType,
  cwd: string
): LockfileValidationCommand {
  switch (pm) {
    case "pnpm": {
      // Offline mode proves the frozen lockfile is accepted before pnpm can
      // fetch missing tarballs into the empty validation store.
      const storeDir = path.join(cwd, ".pnpm-validation-store");
      fs.rmSync(storeDir, { recursive: true, force: true });
      fs.mkdirSync(storeDir, { recursive: true });

      return {
        command:
          "pnpm install --frozen-lockfile --ignore-scripts --offline --store-dir ./.pnpm-validation-store",
        acceptsFailure(result) {
          const output = combinedOutput(result);
          return (
            (output.includes("Lockfile is up to date") ||
              output.includes("Already up to date")) &&
            output.includes("ERR_PNPM_NO_OFFLINE_TARBALL")
          );
        }
      };
    }
    case "npm": {
      return {
        command: "npm ci --dry-run --ignore-scripts --no-audit --no-fund"
      };
    }
    case "yarn": {
      // Yarn Classic has no lockfile-only mode. With an empty offline cache,
      // reaching the fetch step proves resolution accepted the lockfile.
      const cacheFolder = path.join(cwd, ".yarn-offline-cache");
      fs.rmSync(cacheFolder, { recursive: true, force: true });
      fs.mkdirSync(cacheFolder, { recursive: true });

      return {
        command:
          "yarn install --frozen-lockfile --ignore-scripts --offline --cache-folder ./.yarn-offline-cache",
        acceptsFailure(result) {
          const output = combinedOutput(result);
          return (
            output.includes("[2/4] Fetching packages") &&
            output.includes("Can't make a request in offline mode")
          );
        }
      };
    }
    case "yarn-berry": {
      return {
        command: "yarn install --mode=update-lockfile",
        verifyLockfileUnchanged: true
      };
    }
    case "bun": {
      return {
        command: "bun install --frozen-lockfile",
        env: { BUN_CONFIG_SKIP_INSTALL_PACKAGES: "1" }
      };
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

      const validation = lockfileValidationCommand(
        fixture.packageManager,
        tmpDir
      );
      console.log(
        `[${fixture.filename}] Validating fixture (${validation.command})...`
      );
      const validationResult = await this.runLockfileValidation(
        fixture,
        tmpDir,
        fullPath,
        validation
      );

      if (validationResult.error) {
        return (
          `INVALID FIXTURE: lockfile validation fails on unpruned original.\n` +
          `This means the fixture's package.jsons don't match its lockfile.\n` +
          `Fix the fixture or rebuild it from a real repo.\n\n${validationResult.error}`
        );
      }

      console.log(`[${fixture.filename}] Fixture validated`);
      return null;
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  }

  private async runLockfileValidation(
    fixture: TestCase["fixture"],
    cwd: string,
    fullPath: string,
    validation: LockfileValidationCommand
  ): Promise<{ output: string; error: string | null }> {
    const before = readLockfile(cwd, fixture.lockfileName);
    if (before === null) {
      return {
        output: "",
        error: `Lockfile not found before validation: ${fixture.lockfileName}`
      };
    }

    const result = await exec(validation.command, cwd, {
      PATH: fullPath,
      COREPACK_ENABLE_STRICT: "0",
      ...validation.env
    });
    const output = combinedOutput(result);
    const success =
      result.exitCode === 0 || (validation.acceptsFailure?.(result) ?? false);

    if (!success) {
      return {
        output,
        error: `Lockfile validation failed (exit ${result.exitCode}).\n\n${output}`
      };
    }

    if (validation.verifyLockfileUnchanged) {
      const after = readLockfile(cwd, fixture.lockfileName);
      if (after === null) {
        return {
          output,
          error: `Lockfile not found after validation: ${fixture.lockfileName}`
        };
      }

      if (!before.equals(after)) {
        return {
          output,
          error:
            `Lockfile validation changed ${fixture.lockfileName}.\n` +
            "The lockfile is not valid for the current package.json files.\n\n" +
            output
        };
      }
    }

    return { output, error: null };
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
          validationSuccess: false,
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
          validationSuccess: false,
          durationMs: 0
        };

        try {
          // turbo prune
          const pruneCommand = `turbo prune ${targetWorkspace.name}${
            tc.docker ? " --docker" : ""
          }`;
          log(`[${label}] ${pruneCommand}`);
          const pruneResult = await exec(pruneCommand, tmpDir, {
            PATH: fullPath
          });

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

          // Validate the pruned lockfile without downloading packages.
          const outDir = tc.docker
            ? path.join(tmpDir, "out", "json")
            : path.join(tmpDir, "out");
          const outLabel = tc.docker ? "out/json" : "out";
          const validation = lockfileValidationCommand(
            fixture.packageManager,
            outDir
          );
          log(
            `[${label}] ${validation.command} (lockfile validation in ${outLabel}/)`
          );

          const validationResult = await this.runLockfileValidation(
            fixture,
            outDir,
            fullPath,
            validation
          );
          result.validationOutput = validationResult.output;

          if (validationResult.error) {
            log(`[${label}] LOCKFILE VALIDATION FAILED`);
            result.error = validationResult.error;
            result.durationMs = Date.now() - startTime;
            results.push(result);
            continue;
          }

          result.validationSuccess = true;
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
