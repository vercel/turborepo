/**
 * check-examples.ts
 *
 * Validates all examples marked as `maintainedByCoreTeam` in meta.json
 * by running them in isolated Vercel Sandboxes.
 *
 * Usage:
 *   pnpm run check-examples
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

function getExamplesDir(): string {
  return path.dirname(new URL(import.meta.url).pathname);
}

function detectPackageManager(examplePath: string): PackageManagerType {
  if (fs.existsSync(path.join(examplePath, "pnpm-lock.yaml"))) {
    return "pnpm";
  }
  if (fs.existsSync(path.join(examplePath, "yarn.lock"))) {
    return "yarn";
  }
  if (fs.existsSync(path.join(examplePath, "package-lock.json"))) {
    return "npm";
  }

  const packagePath = path.join(examplePath, "package.json");
  if (fs.existsSync(packagePath)) {
    const pkg: PackageJson = JSON.parse(fs.readFileSync(packagePath, "utf-8"));
    if (pkg.packageManager?.startsWith("pnpm")) return "pnpm";
    if (pkg.packageManager?.startsWith("yarn")) return "yarn";
    if (pkg.packageManager?.startsWith("npm")) return "npm";
  }

  return "pnpm";
}

function findMaintainedExamples(examplesDir: string): {
  name: string;
  path: string;
  packageManager: PackageManagerType;
}[] {
  const examples: {
    name: string;
    path: string;
    packageManager: PackageManagerType;
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
        path: examplePath,
        packageManager: detectPackageManager(examplePath)
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
      entry.name === "build"
    ) {
      continue;
    }

    if (entry.isDirectory()) {
      const subFiles = await collectFilesRecursively(fullPath, baseDir);
      files.push(...subFiles);
    } else {
      files.push({
        path: relativePath,
        content: fs.readFileSync(fullPath)
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
const MAX_ERROR_LENGTH = 500;

function truncateError(error: string): string {
  if (error.length <= MAX_ERROR_LENGTH) return error;
  return error.slice(0, MAX_ERROR_LENGTH);
}

async function installTurbo(sandbox: Sandbox, label: string): Promise<void> {
  console.log(`[${label}] Installing turbo...`);
  const result = await sandbox.runCommand("npm", ["install", "-g", "turbo"]);
  if (result.exitCode !== 0) {
    const stderr = await result.stderr();
    throw new Error(`Failed to install turbo: ${stderr}`);
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
  const result = await sandbox.runCommand(packageManager, ["install"], {
    cwd: SANDBOX_CWD
  });
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
    const taskResult = await sandbox.runCommand("turbo", ["run", task], {
      cwd: SANDBOX_CWD
    });

    if (taskResult.exitCode !== 0) {
      const stderr = await taskResult.stderr();
      const stdout = await taskResult.stdout();
      const errorOutput = stderr || stdout;
      console.log(`[${label}] Task ${task} FAILED`);
      success = false;
      results[task] = {
        success: false,
        error: truncateError(errorOutput)
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
    const taskResult = await sandbox.runCommand("turbo", ["run", task], {
      cwd: SANDBOX_CWD
    });

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
    await uploadExampleFiles(sandbox, examplePath, exampleName);
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

  console.log("Finding maintained examples...\n");

  const examplesDir = getExamplesDir();
  const examples = findMaintainedExamples(examplesDir);

  console.log(
    `Found ${examples.length} maintained examples: ${examples.map((e) => e.name).join(", ")}\n`
  );

  const exampleConfigs = examples.map((example) => ({
    ...example,
    tasks: getTasksToRun(example.path)
  }));

  console.log("Running all examples in parallel (fresh sandboxes)...\n");

  const settledResults = await Promise.all(
    exampleConfigs.map(async (example) => {
      const result = await runExample(
        example.name,
        example.path,
        example.packageManager,
        example.tasks
      );
      return { name: example.name, result };
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

  // Print timing for each example
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
