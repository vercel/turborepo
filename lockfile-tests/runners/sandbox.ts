import { Sandbox, Snapshot } from "@vercel/sandbox";
import * as fs from "fs";
import * as path from "path";
import type {
  FixtureInfo,
  TestCase,
  TestResult,
  Runner
} from "../parsers/types";

const SANDBOX_CWD = "/vercel/sandbox";
const COREPACK_BIN = `${SANDBOX_CWD}/.corepack-bin`;
const TURBO_BIN = "/usr/local/bin/turbo";
const REPO_URL = "https://github.com/vercel/turborepo.git";

export interface SandboxRunnerConfig {
  /** Git ref (commit SHA or branch) to clone and build. Falls back to HEAD of main. */
  gitRef?: string;
  /** Reuse an existing snapshot instead of building. */
  snapshotId?: string;
}

function sandboxRun(
  sandbox: Sandbox,
  cmd: string,
  args: string[] = [],
  cwd: string = SANDBOX_CWD
) {
  return sandbox.runCommand({ cmd, args, cwd });
}

function getShellEnv(fixture: FixtureInfo): string {
  if (fixture.packageManager === "bun") {
    return `export PATH="$HOME/.bun/bin:${COREPACK_BIN}:$PATH"`;
  }
  return `export PATH="${COREPACK_BIN}:$PATH"`;
}

async function runShell(
  sandbox: Sandbox,
  shellCommand: string,
  fixture: FixtureInfo,
  cwd: string = SANDBOX_CWD
): Promise<{ exitCode: number; stdout: string; stderr: string }> {
  const envSetup = getShellEnv(fixture);
  const result = await sandboxRun(
    sandbox,
    "sh",
    ["-c", `${envSetup} && ${shellCommand}`],
    cwd
  );
  return {
    exitCode: result.exitCode,
    stdout: await result.stdout(),
    stderr: await result.stderr()
  };
}

function collectFiles(
  dir: string,
  baseDir: string = dir
): { path: string; content: Buffer }[] {
  const files: { path: string; content: Buffer }[] = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.name === "meta.json") continue;
    if (entry.isDirectory()) {
      files.push(...collectFiles(full, baseDir));
    } else {
      files.push({
        path: path.relative(baseDir, full),
        content: fs.readFileSync(full)
      });
    }
  }
  return files;
}

async function execAndCheck(
  sandbox: Sandbox,
  label: string,
  shellCmd: string,
  cwd: string = SANDBOX_CWD,
  env?: Record<string, string>
): Promise<void> {
  const envStr = env
    ? Object.entries(env)
        .map(([k, v]) => `export ${k}="${v}"`)
        .join(" && ") + " && "
    : "";
  const result = await sandboxRun(
    sandbox,
    "sh",
    ["-c", `${envStr}${shellCmd}`],
    cwd
  );
  if (result.exitCode !== 0) {
    const stderr = await result.stderr();
    const stdout = await result.stdout();
    throw new Error(
      `[${label}] Command failed (exit ${result.exitCode}): ${shellCmd}\n${stderr || stdout}`
    );
  }
}

// All pnpm/yarn versions we need for fixtures, so each test sandbox starts ready
const COREPACK_VERSIONS = [
  "pnpm@7.33.0",
  "pnpm@8.15.0",
  "pnpm@9.15.0",
  "pnpm@10.0.0",
  "yarn@3.6.0",
  "yarn@4.1.0"
];

export class SandboxRunner implements Runner {
  private config: SandboxRunnerConfig;
  private snapshotId: string | null = null;
  private ownsSnapshot = false;

  constructor(config: SandboxRunnerConfig = {}) {
    this.config = config;
  }

  async prepare(): Promise<void> {
    if (this.config.snapshotId) {
      console.log(`Using existing snapshot: ${this.config.snapshotId}`);
      this.snapshotId = this.config.snapshotId;
      return;
    }

    console.log("Building turbo in sandbox and creating snapshot...\n");

    const gitRef = this.config.gitRef || "main";
    console.log(`  Git ref: ${gitRef}`);
    console.log(`  Repo: ${REPO_URL}\n`);

    const buildStart = Date.now();

    // Create the build sandbox by cloning the repo
    console.log("  Creating build sandbox (cloning repo)...");
    const sandbox = await Sandbox.create({
      runtime: "node24",
      timeout: 30 * 60 * 1000,
      resources: { vcpus: 4 },
      source: {
        type: "git",
        url: REPO_URL,
        depth: 1,
        revision: gitRef
      }
    });
    console.log(`  Build sandbox ready: ${sandbox.sandboxId}`);

    try {
      // Install Rust toolchain
      console.log("  Installing Rust toolchain...");
      await execAndCheck(
        sandbox,
        "build",
        "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
      );

      // Install protoc (required by turborepo build)
      console.log("  Installing protoc...");
      await execAndCheck(
        sandbox,
        "build",
        [
          'PROTOC_VERSION="26.1"',
          'PROTOC_ARCH="linux-x86_64"',
          'curl -LO "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-${PROTOC_ARCH}.zip"',
          'unzip -o "protoc-${PROTOC_VERSION}-${PROTOC_ARCH}.zip" -d /usr/local',
          "rm -f protoc-*.zip"
        ].join(" && ")
      );

      // Build turbo
      console.log("  Building turbo (cargo build)...");
      await execAndCheck(
        sandbox,
        "build",
        'source "$HOME/.cargo/env" && cargo build'
      );

      // Copy binary to a stable location
      console.log("  Installing turbo binary...");
      await execAndCheck(
        sandbox,
        "build",
        `cp target/debug/turbo ${TURBO_BIN} && chmod +x ${TURBO_BIN}`
      );

      // Verify it works
      await execAndCheck(sandbox, "build", `${TURBO_BIN} --version`);

      // Set up corepack
      console.log("  Setting up corepack...");
      await execAndCheck(
        sandbox,
        "build",
        `mkdir -p ${COREPACK_BIN} && corepack enable --install-directory ${COREPACK_BIN}`
      );

      // Prepare all package manager versions
      console.log("  Preparing package manager versions...");
      for (const pm of COREPACK_VERSIONS) {
        console.log(`    ${pm}`);
        await execAndCheck(
          sandbox,
          "build",
          `export PATH="${COREPACK_BIN}:$PATH" && corepack prepare ${pm} --activate`
        );
      }

      // Install bun
      console.log("  Installing bun...");
      await execAndCheck(
        sandbox,
        "build",
        "curl -fsSL https://bun.sh/install | bash"
      );

      // Set up git config (so test sandboxes don't need to)
      await execAndCheck(
        sandbox,
        "build",
        'git config --global user.email "test@test.com" && git config --global user.name "test"'
      );

      // Clean up build artifacts to reduce snapshot size (keep the binary)
      console.log("  Cleaning up build artifacts...");
      await execAndCheck(
        sandbox,
        "build",
        "rm -rf target/debug/build target/debug/deps target/debug/incremental target/debug/.fingerprint"
      );

      // Take snapshot
      const buildDuration = ((Date.now() - buildStart) / 1000).toFixed(1);
      console.log(
        `\n  Build complete (${buildDuration}s). Creating snapshot...`
      );
      const snapshot = await sandbox.snapshot(
        { expiration: 24 * 60 * 60 * 1000 } as any // 24 hours; SDK types may lag behind API
      );

      this.snapshotId = snapshot.snapshotId;
      this.ownsSnapshot = true;
      console.log(`  Snapshot created: ${this.snapshotId}\n`);

      // Write snapshot ID to disk for CI cache
      const snapshotIdPath = path.join(
        path.dirname(new URL(import.meta.url).pathname),
        "..",
        ".snapshot-id"
      );
      fs.writeFileSync(snapshotIdPath, this.snapshotId);
    } catch (err) {
      // Snapshot wasn't taken, need to stop manually
      await sandbox.stop();
      throw err;
    }
    // Note: sandbox.snapshot() automatically stops the sandbox
  }

  async cleanup(): Promise<void> {
    if (this.snapshotId && this.ownsSnapshot) {
      console.log(`\nDeleting snapshot: ${this.snapshotId}`);
      try {
        const snapshot = await Snapshot.get({ snapshotId: this.snapshotId });
        await snapshot.delete();
        console.log("Snapshot deleted.");
      } catch (err) {
        console.warn(`Failed to delete snapshot: ${err}`);
      }
    }
  }

  async runTestCase(
    testCase: TestCase,
    _turboBinaryPath: string
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

    if (!this.snapshotId) {
      result.error = "No snapshot available. Did prepare() run?";
      result.durationMs = Date.now() - startTime;
      return result;
    }

    let sandbox: Sandbox | null = null;

    try {
      log(`[${label}] Creating sandbox from snapshot...`);
      sandbox = await Sandbox.create({
        runtime: "node24",
        timeout: 10 * 60 * 1000,
        resources: { vcpus: 4 },
        source: {
          type: "snapshot",
          snapshotId: this.snapshotId
        }
      });
      log(`[${label}] Sandbox ready: ${sandbox.sandboxId}`);

      // Upload fixture files (small — just package.jsons + lockfile)
      log(`[${label}] Uploading fixture...`);
      const fixtureFiles = collectFiles(fixture.filepath);
      await sandbox.writeFiles(fixtureFiles);

      // Git init for the fixture
      await sandboxRun(sandbox, "sh", [
        "-c",
        "git init && git add . && git commit -m init"
      ]);

      // turbo prune
      log(`[${label}] turbo prune ${targetWorkspace.name}...`);
      const pruneResult = await runShell(
        sandbox,
        `turbo prune ${targetWorkspace.name}`,
        fixture
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

      // Frozen install
      const outDir = `${SANDBOX_CWD}/out`;
      let installCmd = fixture.frozenInstallCommand.join(" ");

      if (fixture.packageManager === "bun") {
        const bunArgs = fixture.frozenInstallCommand.slice(1).join(" ");
        installCmd = `bun ${bunArgs}`;
      }

      log(`[${label}] ${installCmd} (in out/)...`);
      const installResult = await runShell(
        sandbox,
        installCmd,
        fixture,
        outDir
      );

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
      if (sandbox) {
        log(`[${label}] Stopping sandbox...`);
        await sandbox.stop();
      }
      result.durationMs = Date.now() - startTime;
    }

    return result;
  }
}
