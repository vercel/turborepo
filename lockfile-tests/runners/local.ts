import { exec as execCb } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { TestCase, TestResult } from "../types";

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

interface SetupResult {
  installCmd: string;
  fullPath: string;
}

export class LocalRunner {
  // Tracks which fixtures have been validated (keyed by filepath)
  private validated = new Map<string, string | null>();
  // In-flight validation promises so concurrent tests for the same fixture wait
  private validating = new Map<string, Promise<string | null>>();

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
      const { installCmd, fullPath } = await this.setupEnv(
        fixture,
        tmpDir,
        turboBinaryPath,
        console.log
      );

      console.log(`[${fixture.filename}] Validating fixture...`);
      const result = await exec(installCmd, tmpDir, {
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
   * Sets up the environment in a temp dir: corepack, bun, PATH.
   * Returns the install command and PATH to use.
   */
  private async setupEnv(
    fixture: TestCase["fixture"],
    tmpDir: string,
    turboBinaryPath: string,
    log: (...args: unknown[]) => void
  ): Promise<SetupResult> {
    const localBin = path.join(tmpDir, ".bin");
    fs.mkdirSync(localBin, { recursive: true });
    if (turboBinaryPath) {
      fs.symlinkSync(turboBinaryPath, path.join(localBin, "turbo"));
    }

    const corepackBin = path.join(tmpDir, ".corepack-bin");
    fs.mkdirSync(corepackBin, { recursive: true });
    await exec(`corepack enable --install-directory "${corepackBin}"`, tmpDir);
    let fullPath = `${corepackBin}:${localBin}:${process.env.PATH}`;

    let installCmd = fixture.frozenInstallCommand.join(" ");

    if (fixture.packageManager === "bun") {
      const bunVersion = fixture.packageManagerVersion.replace("bun@", "");
      const bunDir = path.join(tmpDir, ".bun-install");

      log(`[${fixture.filename}] Installing bun@${bunVersion}`);
      const bunInstall = await exec(
        `curl -fsSL https://bun.sh/install | BUN_INSTALL="${bunDir}" bash -s "bun-v${bunVersion}"`,
        tmpDir,
        { PATH: fullPath }
      );
      if (bunInstall.exitCode !== 0) {
        throw new Error(
          `Failed to install bun@${bunVersion}: ${bunInstall.stderr}`
        );
      }
      fullPath = `${bunDir}/bin:${fullPath}`;
    } else {
      log(
        `[${fixture.filename}] corepack prepare ${fixture.packageManagerVersion}`
      );
      const prep = await exec(
        `corepack prepare ${fixture.packageManagerVersion} --activate`,
        tmpDir,
        { PATH: fullPath, COREPACK_ENABLE_STRICT: "0" }
      );
      if (prep.exitCode !== 0) {
        log(
          `[${fixture.filename}] corepack prepare warning: ${prep.stderr || prep.stdout}`
        );
      }
    }

    return { installCmd, fullPath };
  }

  async runTestCase(
    testCase: TestCase,
    turboBinaryPath: string
  ): Promise<TestResult> {
    const { fixture, targetWorkspace, label, expectedFailure } = testCase;
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

    // Validate once per fixture
    const validationError = await this.validateFixture(
      fixture,
      turboBinaryPath
    );
    if (validationError) {
      result.error = validationError;
      result.durationMs = Date.now() - startTime;
      return result;
    }

    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lockfile-test-"));

    try {
      log(`[${label}] Copying fixture to ${tmpDir}`);
      copyDirSync(fixture.filepath, tmpDir);

      const { installCmd, fullPath } = await this.setupEnv(
        fixture,
        tmpDir,
        turboBinaryPath,
        log
      );

      // Git init (turbo requires it)
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
        return result;
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
        return result;
      }

      result.installSuccess = true;
      result.success = true;
      log(`[${label}] PASSED`);
    } catch (err) {
      result.error = err instanceof Error ? err.message : String(err);
      if (!quiet) console.error(`[${label}] ERROR: ${result.error}`);
    } finally {
      try {
        fs.rmSync(tmpDir, { recursive: true, force: true });
      } catch {
        // best-effort
      }
      result.durationMs = Date.now() - startTime;
    }

    return result;
  }
}
