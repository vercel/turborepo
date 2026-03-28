/**
 * check-examples.ts
 *
 * Validates all examples marked as `maintainedByCoreTeam` in meta.json
 * by running them in isolated Vercel Sandboxes.
 *
 * Usage:
 *   pnpm check-examples [--example <name>] [--pm <pnpm|npm|yarn>]
 *
 * Examples:
 *   pnpm check-examples                           # Run all examples with all package managers
 *   pnpm check-examples --example basic           # Run only the "basic" example with all PMs
 *   pnpm check-examples --pm pnpm                 # Run all examples with pnpm only
 *   pnpm check-examples --example basic --pm npm  # Run "basic" with npm only
 */

import { Sandbox } from "@vercel/sandbox";
import * as fs from "fs";
import * as path from "path";

interface ExampleMeta {
  name: string;
  description: string;
  maintainedByCoreTeam: boolean;
  template?: string;
}

interface TurboConfig {
  tasks?: Record<string, unknown>;
}

interface PackageJson {
  scripts?: Record<string, string>;
  packageManager?: string;
}

interface TaskResult {
  success: boolean;
  error?: string;
  cached?: boolean;
}

interface ExampleResult {
  success: boolean;
  tasks: Record<string, TaskResult>;
  cacheVerification: Record<string, TaskResult>;
  durationMs: number;
}

type Results = Record<string, ExampleResult>;

type PackageManagerType = "pnpm" | "npm" | "yarn";

const ALL_PACKAGE_MANAGERS: PackageManagerType[] = ["pnpm", "npm", "yarn"];

interface CliArgs {
  example?: string;
  pm?: PackageManagerType;
}

function parseArgs(): CliArgs {
  const args: CliArgs = {};
  const argv = process.argv.slice(2);

  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--example" && argv[i + 1]) {
      args.example = argv[++i];
    } else if (argv[i] === "--pm" && argv[i + 1]) {
      const pm = argv[++i];
      if (pm === "pnpm" || pm === "npm" || pm === "yarn") {
        args.pm = pm;
      } else {
        console.error(
          `Invalid package manager: ${pm}. Must be pnpm, npm, or yarn.`
        );
        process.exit(1);
      }
    }
  }

  return args;
}

function getExamplesDir(): string {
  return path.dirname(new URL(import.meta.url).pathname);
}

function findMaintainedExamples(examplesDir: string): {
  name: string;
  path: string;
}[] {
  const examples: {
    name: string;
    path: string;
  }[] = [];
  const entries = fs.readdirSync(examplesDir, { withFileTypes: true });

  for (const entry of entries) {
    if (!entry.isDirectory()) continue;

    const metaPath = path.join(examplesDir, entry.name, "meta.json");
    if (!fs.existsSync(metaPath)) continue;

    const meta: ExampleMeta = JSON.parse(fs.readFileSync(metaPath, "utf-8"));
    if (meta.maintainedByCoreTeam) {
      const examplePath = path.join(examplesDir, entry.name);
      examples.push({
        name: entry.name,
        path: examplePath
      });
    }
  }

  return examples;
}

function getTasksToRun(examplePath: string): string[] {
  const turboPath = path.join(examplePath, "turbo.json");

  const tasks: string[] = [];

  if (fs.existsSync(turboPath)) {
    const turboConfig: TurboConfig = JSON.parse(
      fs.readFileSync(turboPath, "utf-8")
    );
    if (turboConfig.tasks) {
      for (const [taskName, taskConfig] of Object.entries(turboConfig.tasks)) {
        const config = taskConfig as { persistent?: boolean; cache?: boolean };
        if (!config.persistent && taskName !== "dev" && taskName !== "clean") {
          tasks.push(taskName);
        }
      }
    }
  }

  return tasks;
}

async function collectFilesRecursively(
  dir: string,
  baseDir: string = dir
): Promise<{ path: string; content: Buffer }[]> {
  const files: { path: string; content: Buffer }[] = [];
  const entries = fs.readdirSync(dir, { withFileTypes: true });

  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    const relativePath = path.relative(baseDir, fullPath);

    if (
      entry.name === "node_modules" ||
      entry.name === ".git" ||
      entry.name === ".turbo" ||
      entry.name === ".next" ||
      entry.name === "dist" ||
      entry.name === "build" ||
      entry.name === "pnpm-lock.yaml" ||
      entry.name === "package-lock.json" ||
      entry.name === "yarn.lock"
    ) {
      continue;
    }

    if (entry.isDirectory()) {
      const subFiles = await collectFilesRecursively(fullPath, baseDir);
      files.push(...subFiles);
    } else {
      const content = fs.readFileSync(fullPath);
      files.push({
        path: relativePath,
        content
      });
    }
  }

  return files;
}

function checkCacheHit(output: string): boolean {
  // Turbo outputs "FULL TURBO" or shows cache hit status
  // When all tasks hit cache, output contains ">>> FULL TURBO"
  // Individual cached tasks show "cache hit" or "CACHED"
  return (
    output.includes("FULL TURBO") ||
    output.includes("cache hit, replaying logs")
  );
}

const SANDBOX_CWD = "/vercel/sandbox";

async function installTurbo(sandbox: Sandbox, label: string): Promise<void> {
  console.log(`[${label}] Installing turbo...`);
  const result = await sandbox.runCommand("npm", ["install", "-g", "turbo"]);
  if (result.exitCode !== 0) {
    const stderr = await result.stderr();
    throw new Error(`Failed to install turbo: ${stderr}`);
  }
}

const COREPACK_BIN = `${SANDBOX_CWD}/.corepack-bin`;

async function enableCorepack(sandbox: Sandbox, label: string): Promise<void> {
  console.log(`[${label}] Enabling corepack...`);

  // Create a bin directory in the sandbox that we can write to
  await sandbox.runCommand("mkdir", ["-p", COREPACK_BIN]);

  const result = await sandbox.runCommand("corepack", [
    "enable",
    "--install-directory",
    COREPACK_BIN
  ]);
  if (result.exitCode !== 0) {
    const stderr = await result.stderr();
    const stdout = await result.stdout();
    throw new Error(
      `Failed to enable corepack (exit ${result.exitCode}): ${stderr || stdout || "no output"}`
    );
  }
}

async function convertToPackageManager(
  sandbox: Sandbox,
  packageManager: PackageManagerType,
  label: string
): Promise<void> {
  console.log(`[${label}] Converting to ${packageManager}...`);
  // Run with PATH including corepack binaries so yarn/pnpm are available
  const result = await sandbox.runCommand(
    "sh",
    [
      "-c",
      `export PATH="${COREPACK_BIN}:$PATH" && npx @turbo/workspaces convert . ${packageManager} --skip-install --ignore-unchanged-package-manager`
    ],
    { cwd: SANDBOX_CWD }
  );
  if (result.exitCode !== 0) {
    const stderr = await result.stderr();
    const stdout = await result.stdout();
    throw new Error(
      `Failed to convert to ${packageManager} (exit ${result.exitCode}):\nstderr: ${stderr}\nstdout: ${stdout}`
    );
  }
}

async function uploadExampleFiles(
  sandbox: Sandbox,
  examplePath: string,
  label: string
): Promise<void> {
  console.log(`[${label}] Uploading files...`);
  const files = await collectFilesRecursively(examplePath);
  await sandbox.writeFiles(files);
}

async function installDependencies(
  sandbox: Sandbox,
  packageManager: PackageManagerType,
  label: string
): Promise<void> {
  console.log(`[${label}] Installing dependencies (${packageManager})...`);
  // Run with PATH including corepack binaries so yarn/pnpm are available
  const result = await sandbox.runCommand(
    "sh",
    ["-c", `export PATH="${COREPACK_BIN}:$PATH" && ${packageManager} install`],
    { cwd: SANDBOX_CWD }
  );
  if (result.exitCode !== 0) {
    const stderr = await result.stderr();
    throw new Error(`Failed to install dependencies: ${stderr}`);
  }
}

async function runTasks(
  sandbox: Sandbox,
  tasks: string[],
  label: string
): Promise<{ success: boolean; results: Record<string, TaskResult> }> {
  const results: Record<string, TaskResult> = {};
  let success = true;

  for (const task of tasks) {
    console.log(`[${label}] Running: ${task}...`);
    const taskResult = await sandbox.runCommand(
      "sh",
      ["-c", `export PATH="${COREPACK_BIN}:$PATH" && turbo run ${task}`],
      { cwd: SANDBOX_CWD }
    );

    if (taskResult.exitCode !== 0) {
      const stderr = await taskResult.stderr();
      const stdout = await taskResult.stdout();
      // Combine both streams to get full picture of what happened
      const fullOutput = [stdout, stderr].filter(Boolean).join("\n");
      console.log(`[${label}] Task ${task} FAILED`);
      success = false;
      results[task] = {
        success: false,
        error: fullOutput
      };
    } else {
      console.log(`[${label}] Task ${task} passed`);
      results[task] = { success: true };
    }
  }

  return { success, results };
}

async function verifyCacheHits(
  sandbox: Sandbox,
  tasks: string[],
  label: string
): Promise<{ success: boolean; results: Record<string, TaskResult> }> {
  console.log(`[${label}] Verifying cache hits...`);
  const results: Record<string, TaskResult> = {};
  let success = true;

  for (const task of tasks) {
    console.log(`[${label}] Cache check: ${task}...`);
    const taskResult = await sandbox.runCommand(
      "sh",
      ["-c", `export PATH="${COREPACK_BIN}:$PATH" && turbo run ${task}`],
      { cwd: SANDBOX_CWD }
    );

    const stdout = await taskResult.stdout();
    const stderr = await taskResult.stderr();
    const output = stdout + stderr;

    if (taskResult.exitCode !== 0) {
      console.log(`[${label}] Cache verify ${task} FAILED (command error)`);
      success = false;
      results[task] = {
        success: false,
        cached: false,
        error: "Command failed on second run"
      };
    } else if (!checkCacheHit(output)) {
      console.log(`[${label}] Cache verify ${task} FAILED (no cache hit)`);
      success = false;
      results[task] = {
        success: false,
        cached: false,
        error: "Expected cache hit but task ran again"
      };
    } else {
      console.log(`[${label}] Cache verify ${task} OK`);
      results[task] = { success: true, cached: true };
    }
  }

  return { success, results };
}

async function runExample(
  exampleName: string,
  examplePath: string,
  packageManager: PackageManagerType,
  tasks: string[]
): Promise<ExampleResult> {
  const startTime = Date.now();
  const result: ExampleResult = {
    success: true,
    tasks: {},
    cacheVerification: {},
    durationMs: 0
  };

  console.log(`[${exampleName}] Starting fresh sandbox...`);

  let sandbox: Sandbox | null = null;

  try {
    sandbox = await Sandbox.create({
      runtime: "node24",
      timeout: 10 * 60 * 1000,
      resources: { vcpus: 4 }
    });
    console.log(`[${exampleName}] Sandbox created: ${sandbox.sandboxId}`);

    await installTurbo(sandbox, exampleName);
    await enableCorepack(sandbox, exampleName);
    await uploadExampleFiles(sandbox, examplePath, exampleName);
    await convertToPackageManager(sandbox, packageManager, exampleName);
    await installDependencies(sandbox, packageManager, exampleName);
    result.tasks["install"] = { success: true };

    const taskRun = await runTasks(sandbox, tasks, exampleName);
    Object.assign(result.tasks, taskRun.results);
    result.success = taskRun.success;

    if (result.success && tasks.length > 0) {
      const cacheRun = await verifyCacheHits(sandbox, tasks, exampleName);
      result.cacheVerification = cacheRun.results;
      result.success = cacheRun.success;
    }
  } catch (error) {
    console.error(`[${exampleName}] Error: ${error}`);
    result.success = false;
    result.tasks["sandbox"] = {
      success: false,
      error: error instanceof Error ? error.message : String(error)
    };
  } finally {
    if (sandbox) {
      console.log(`[${exampleName}] Stopping sandbox...`);
      await sandbox.stop();
    }
    result.durationMs = Date.now() - startTime;
    console.log(
      `[${exampleName}] Done (${(result.durationMs / 1000).toFixed(1)}s)`
    );
  }

  return result;
}

async function main(): Promise<void> {
  const totalStartTime = Date.now();
  const cliArgs = parseArgs();

  console.log("Finding maintained examples...\n");

  const examplesDir = getExamplesDir();
  let examples = findMaintainedExamples(examplesDir);

  // Filter to specific example if requested
  if (cliArgs.example) {
    const filtered = examples.filter((e) => e.name === cliArgs.example);
    if (filtered.length === 0) {
      console.error(`Example not found: ${cliArgs.example}`);
      console.error(
        `Available examples: ${examples.map((e) => e.name).join(", ")}`
      );
      process.exit(1);
    }
    examples = filtered;
  }

  // Determine which package managers to test
  const packageManagers: PackageManagerType[] = cliArgs.pm
    ? [cliArgs.pm]
    : ALL_PACKAGE_MANAGERS;

  console.log(
    `Found ${examples.length} maintained examples: ${examples.map((e) => e.name).join(", ")}\n`
  );
  console.log(
    `Testing each with ${packageManagers.length} package managers: ${packageManagers.join(", ")}\n`
  );

  const exampleConfigs: {
    name: string;
    path: string;
    packageManager: PackageManagerType;
    tasks: string[];
  }[] = [];

  for (const example of examples) {
    const tasks = getTasksToRun(example.path);
    for (const pm of packageManagers) {
      exampleConfigs.push({
        name: example.name,
        path: example.path,
        packageManager: pm,
        tasks
      });
    }
  }

  console.log(
    `Running ${exampleConfigs.length} test combinations in parallel...\n`
  );

  const settledResults = await Promise.all(
    exampleConfigs.map(async (example) => {
      const label = `${example.name} (${example.packageManager})`;
      const result = await runExample(
        label,
        example.path,
        example.packageManager,
        example.tasks
      );
      return { name: label, result };
    })
  );

  const results: Results = {};
  for (const { name, result } of settledResults) {
    results[name] = result;
  }

  const totalDurationMs = Date.now() - totalStartTime;

  console.log("\n" + "=".repeat(50));
  console.log("Results Summary");
  console.log("=".repeat(50) + "\n");

  for (const [name, result] of Object.entries(results)) {
    const status = result.success ? "PASS" : "FAIL";
    const cacheStatus = Object.values(result.cacheVerification).every(
      (t) => t.cached
    )
      ? "cache OK"
      : "cache MISS";
    console.log(
      `${name}: ${status} (${(result.durationMs / 1000).toFixed(1)}s) [${cacheStatus}]`
    );
  }

  console.log(`\nTotal time: ${(totalDurationMs / 1000).toFixed(1)}s`);

  console.log("\nDetailed results:");
  console.log(JSON.stringify(results, null, 2));

  const allPassed = Object.values(results).every((r) => r.success);
  if (!allPassed) {
    console.log("\nSome examples failed!");
    process.exit(1);
  } else {
    console.log("\nAll examples passed!");
  }
}

main().catch((error) => {
  console.error("Fatal error:", error);
  process.exit(1);
});
