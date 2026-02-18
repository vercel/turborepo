import { exec as execCb } from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
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
        timeout: 5 * 60 * 1000,
        maxBuffer: 10 * 1024 * 1024
      },
      (error, stdout, stderr) => {
        if (error) {
          // error.code is the exit code (number at runtime, typed as string)
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
      log(`[${label}] Copying fixture to ${tmpDir}`);
      copyDirSync(fixture.filepath, tmpDir);

      // Put turbo on PATH via symlink
      const localBin = path.join(tmpDir, ".bin");
      fs.mkdirSync(localBin);
      fs.symlinkSync(turboBinaryPath, path.join(localBin, "turbo"));

      // Set up corepack
      const corepackBin = path.join(tmpDir, ".corepack-bin");
      fs.mkdirSync(corepackBin);
      await exec(
        `corepack enable --install-directory "${corepackBin}"`,
        tmpDir
      );
      let fullPath = `${corepackBin}:${localBin}:${process.env.PATH}`;

      // Compute the install command (used for both validation and post-prune)
      let installCmd = fixture.frozenInstallCommand.join(" ");
      if (fixture.packageManager === "bun") {
        const bunVersion = fixture.packageManagerVersion.replace("bun@", "");
        const bunDir = path.join(tmpDir, ".bun-install");

        log(`[${label}] Installing bun@${bunVersion}`);
        const bunInstall = await exec(
          `curl -fsSL https://bun.sh/install | BUN_INSTALL="${bunDir}" bash -s "bun-v${bunVersion}"`,
          tmpDir,
          { PATH: fullPath }
        );
        if (bunInstall.exitCode !== 0) {
          result.error = `Failed to install bun@${bunVersion}: ${bunInstall.stderr}`;
          return result;
        }

        // Put the installed bun at the front of PATH
        fullPath = `${bunDir}/bin:${fullPath}`;

        // Verify it's the right version
        const versionCheck = await exec("bun --version", tmpDir, {
          PATH: fullPath
        });
        log(`[${label}] bun version: ${versionCheck.stdout.trim()}`);
      } else {
        log(`[${label}] corepack prepare ${fixture.packageManagerVersion}`);
        const prep = await exec(
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

      // Validate the unpruned fixture: frozen install must work on the
      // original before we trust it as a test case. If this fails, the
      // fixture's package.jsons don't match its lockfile — either the
      // fixture was generated incorrectly or it needs to be rebuilt from
      // a real repo.
      log(`[${label}] Validating fixture (frozen install on original)...`);
      const validateResult = await exec(installCmd, tmpDir, {
        PATH: fullPath,
        COREPACK_ENABLE_STRICT: "0"
      });
      if (validateResult.exitCode !== 0) {
        const output = [validateResult.stdout, validateResult.stderr]
          .filter(Boolean)
          .join("\n");
        result.error =
          `INVALID FIXTURE: frozen install fails on unpruned original.\n` +
          `This means the fixture's package.jsons don't match its lockfile.\n` +
          `Fix the fixture or rebuild it from a real repo.\n\n${output}`;
        // For expected failures, this is a known issue with parser-generated
        // fixtures. For unexpected tests, this is a hard error.
        return result;
      }
      log(`[${label}] Fixture validated`);
      // Clean up node_modules from validation so turbo prune sees a clean tree
      await exec("rm -rf node_modules .pnp* .yarn/cache .bun", tmpDir);
      // Also clean workspace node_modules
      const cleanDirs = async (dir: string) => {
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
          if (!entry.isDirectory()) continue;
          if (entry.name === "node_modules") {
            fs.rmSync(path.join(dir, entry.name), {
              recursive: true,
              force: true
            });
          } else if (
            entry.name !== ".bin" &&
            entry.name !== ".corepack-bin" &&
            entry.name !== ".git" &&
            entry.name !== "out"
          ) {
            await cleanDirs(path.join(dir, entry.name));
          }
        }
      };
      await cleanDirs(tmpDir);

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
