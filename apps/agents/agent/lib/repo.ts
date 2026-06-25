import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, readdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";

export type JsonObject = Record<string, unknown>;

export interface CommandResult {
  command: string;
  cwd: string;
  exitCode: number | null;
  timedOut: boolean;
  stdout: string;
  stderr: string;
}

const MAX_OUTPUT_LENGTH = 20_000;
const lockfileNames = new Set([
  "bun.lock",
  "bun.lockb",
  "package-lock.json",
  "pnpm-lock.yaml",
  "yarn.lock"
]);

export async function getRepoRoot(): Promise<string> {
  if (process.env.TURBO_REPO_ROOT) {
    return process.env.TURBO_REPO_ROOT;
  }

  let current = process.cwd();
  while (true) {
    const packageJsonPath = path.join(current, "package.json");
    const examplesPath = path.join(current, "examples");
    if (existsSync(packageJsonPath) && existsSync(examplesPath)) {
      const packageJson = await readJsonFile(packageJsonPath);
      if (packageJson.name === "turbo-monorepo") {
        return current;
      }
    }

    const parent = path.dirname(current);
    if (parent === current) {
      throw new Error("Could not locate the Turborepo repository root.");
    }
    current = parent;
  }
}

export async function listExampleNames(): Promise<string[]> {
  const repoRoot = await getRepoRoot();
  const examplesRoot = path.join(repoRoot, "examples");
  const entries = await readdir(examplesRoot, { withFileTypes: true });
  const names = await Promise.all(
    entries
      .filter((entry) => entry.isDirectory())
      .map(async (entry) => {
        const packageJsonPath = path.join(
          examplesRoot,
          entry.name,
          "package.json"
        );
        return existsSync(packageJsonPath) ? entry.name : null;
      })
  );

  return names.filter((name): name is string => name !== null).sort();
}

export async function getExamplePath(example: string): Promise<string> {
  if (example.includes("/") || example.includes("\\") || example === "..") {
    throw new Error(`Invalid example name: ${example}`);
  }

  const examples = await listExampleNames();
  if (!examples.includes(example)) {
    throw new Error(`Unknown example: ${example}`);
  }

  const repoRoot = await getRepoRoot();
  return path.join(repoRoot, "examples", example);
}

export async function readJsonFile(filePath: string): Promise<JsonObject> {
  const content = await readFile(filePath, "utf8");
  const value: unknown = JSON.parse(content);
  if (!isJsonObject(value)) {
    throw new Error(`${filePath} must contain a JSON object.`);
  }
  return value;
}

export async function readTextIfExists(
  filePath: string,
  maxLines = 120
): Promise<string | null> {
  if (!existsSync(filePath)) {
    return null;
  }

  const content = await readFile(filePath, "utf8");
  return content.split("\n").slice(0, maxLines).join("\n");
}

export async function findPackageJsonFiles(root: string): Promise<string[]> {
  const results: string[] = [];

  async function walk(directory: string): Promise<void> {
    const entries = await readdir(directory, { withFileTypes: true });
    await Promise.all(
      entries.map(async (entry) => {
        if (
          entry.name === "node_modules" ||
          entry.name === ".turbo" ||
          entry.name === "dist"
        ) {
          return;
        }

        const entryPath = path.join(directory, entry.name);
        if (entry.isDirectory()) {
          await walk(entryPath);
          return;
        }

        if (entry.isFile() && entry.name === "package.json") {
          results.push(entryPath);
        }
      })
    );
  }

  await walk(root);
  return results.sort();
}

export function packageManagerName(packageManager: unknown): string | null {
  if (typeof packageManager !== "string") {
    return null;
  }
  return packageManager.split("@")[0] ?? null;
}

export async function resolveExamplesFile(
  relativePath: string
): Promise<string> {
  const repoRoot = await getRepoRoot();
  if (path.isAbsolute(relativePath)) {
    throw new Error("Use a repository-relative path under examples/.");
  }

  const resolved = path.resolve(repoRoot, relativePath);
  const relative = path.relative(repoRoot, resolved);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error("Path must stay inside the repository.");
  }
  if (relative !== "examples" && !relative.startsWith(`examples${path.sep}`)) {
    throw new Error(
      "Only files under examples/ can be modified by this agent."
    );
  }

  return resolved;
}

export async function writeExamplesFile(
  relativePath: string,
  content: string
): Promise<{ path: string; bytes: number }> {
  const filePath = await resolveExamplesFile(relativePath);
  const fileStat = existsSync(filePath) ? await stat(filePath) : null;
  if (fileStat?.isDirectory()) {
    throw new Error("Cannot overwrite a directory.");
  }
  if (lockfileNames.has(path.basename(filePath))) {
    throw new Error(
      "Lockfiles must be updated by running the package manager, not by direct writes."
    );
  }

  await mkdir(path.dirname(filePath), { recursive: true });
  await writeFile(filePath, content, "utf8");
  return {
    path: path.relative(await getRepoRoot(), filePath),
    bytes: Buffer.byteLength(content)
  };
}

export async function runCommand(
  command: string,
  args: string[],
  cwd: string,
  timeoutMs: number
): Promise<CommandResult> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, shell: false });
    const commandLine = [command, ...args].join(" ");
    let stdout = "";
    let stderr = "";
    let timedOut = false;

    const timeout = setTimeout(() => {
      timedOut = true;
      child.kill("SIGTERM");
    }, timeoutMs);

    child.stdout.on("data", (chunk: Buffer) => {
      stdout = truncateOutput(stdout + chunk.toString("utf8"));
    });

    child.stderr.on("data", (chunk: Buffer) => {
      stderr = truncateOutput(stderr + chunk.toString("utf8"));
    });

    child.on("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });

    child.on("close", (exitCode) => {
      clearTimeout(timeout);
      resolve({
        command: commandLine,
        cwd,
        exitCode,
        timedOut,
        stdout,
        stderr
      });
    });
  });
}

export function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function pickJsonObject(value: unknown): JsonObject | null {
  return isJsonObject(value) ? value : null;
}

function truncateOutput(output: string): string {
  if (output.length <= MAX_OUTPUT_LENGTH) {
    return output;
  }
  return `${output.slice(0, MAX_OUTPUT_LENGTH)}\n[output truncated]`;
}
