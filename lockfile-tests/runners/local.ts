import { execSync } from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import type { TestCase, TestResult } from "../parsers/types";

function exec(
  command: string,
  cwd: string,
  env?: Record<string, string>
): { exitCode: number; stdout: string; stderr: string } {
  try {
    const stdout = execSync(command, {
      cwd,
      env: { ...process.env, ...env },
      stdio: ["pipe", "pipe", "pipe"],
      timeout: 5 * 60 * 1000,
      maxBuffer: 10 * 1024 * 1024
    });
    return { exitCode: 0, stdout: stdout.toString(), stderr: "" };
  } catch (err: unknown) {
    const e = err as { status?: number; stdout?: Buffer; stderr?: Buffer };
    return {
      exitCode: e.status ?? 1,
      stdout: e.stdout?.toString() ?? "",
      stderr: e.stderr?.toString() ?? ""
    };
  }
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

export class LocalRunner {
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

    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lockfile-test-"));

    try {
      // Copy committed fixture into temp dir
      log(`[${label}] Copying fixture to ${tmpDir}`);
      copyDirSync(fixture.filepath, tmpDir);

      // Put turbo on PATH via symlink
      const localBin = path.join(tmpDir, ".bin");
      fs.mkdirSync(localBin);
      fs.symlinkSync(turboBinaryPath, path.join(localBin, "turbo"));

      // Set up corepack
      const corepackBin = path.join(tmpDir, ".corepack-bin");
      fs.mkdirSync(corepackBin);
      exec(`corepack enable --install-directory "${corepackBin}"`, tmpDir);
      const fullPath = `${corepackBin}:${localBin}:${process.env.PATH}`;

      if (fixture.packageManager === "bun") {
        // Bun doesn't use corepack. Install the specific version into a local dir
        // so we can put it on PATH.
        const bunVersion = fixture.packageManagerVersion.replace("bun@", "");
        log(`[${label}] Installing bun@${bunVersion}`);
        const bunInstall = exec(
          `bunx --bun bun@${bunVersion} --version`,
          tmpDir,
          { PATH: fullPath }
        );
        if (bunInstall.exitCode !== 0) {
          log(`[${label}] bun install warning: ${bunInstall.stderr}`);
        }
      } else {
        log(`[${label}] corepack prepare ${fixture.packageManagerVersion}`);
        const prep = exec(
          `corepack prepare ${fixture.packageManagerVersion} --activate`,
          tmpDir,
          { PATH: fullPath, COREPACK_ENABLE_STRICT: "0" }
        );
        if (prep.exitCode !== 0) {
          log(
            `[${label}] corepack prepare warning: ${prep.stderr || prep.stdout}`
          );
        }
      }

      // Git init (turbo requires it)
      exec(
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
      const pruneResult = exec(`turbo prune ${targetWorkspace.name}`, tmpDir, {
        PATH: fullPath
      });

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
      let installCmd = fixture.frozenInstallCommand.join(" ");

      // For bun, use bunx to run the specific version
      if (fixture.packageManager === "bun") {
        const bunVersion = fixture.packageManagerVersion.replace("bun@", "");
        const bunArgs = fixture.frozenInstallCommand.slice(1).join(" ");
        installCmd = `bunx --bun bun@${bunVersion} ${bunArgs}`;
      }

      log(`[${label}] ${installCmd} (in out/)`);

      const installResult = exec(installCmd, outDir, {
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
