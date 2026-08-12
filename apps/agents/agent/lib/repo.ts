import path from "node:path/posix";

import type { SandboxSession } from "eve/sandbox";

export type JsonObject = Record<string, unknown>;

export interface CommandResult {
  command: string;
  cwd: string;
  exitCode: number;
  timedOut: boolean;
  stdout: string;
  stderr: string;
}

export interface DirectoryEntry {
  name: string;
  type: "directory" | "file" | "other";
}

interface AppPrincipal {
  attributes?: Readonly<Record<string, string | readonly string[]>>;
  authenticator: string;
  principalId: string;
  principalType: string;
}

const REPO_ROOT = "turborepo";
const MAX_OUTPUT_LENGTH = 20_000;
const MILLISECONDS_PER_DAY = 86_400_000;
const ULID_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const lockfileNames = new Set([
  "bun.lock",
  "bun.lockb",
  "package-lock.json",
  "pnpm-lock.yaml",
  "yarn.lock"
]);

export async function getRepoRoot(sandbox: SandboxSession): Promise<string> {
  const result = await sandbox.run({
    command: "test -f package.json && test -d examples",
    workingDirectory: REPO_ROOT
  });
  if (result.exitCode !== 0) {
    throw new Error("The Turborepo sandbox checkout is unavailable.");
  }
  return REPO_ROOT;
}

export async function listExampleNames(
  sandbox: SandboxSession
): Promise<string[]> {
  const repoRoot = await getRepoRoot(sandbox);
  const files = await listTrackedFiles(sandbox, "examples");
  return [
    ...new Set(
      files.flatMap((file) => {
        const segments = file.split("/");
        return segments.length === 3 && segments[2] === "package.json"
          ? [segments[1] as string]
          : [];
      })
    )
  ].sort();
}

export function selectDailyExample(
  examples: string[],
  date = new Date()
): { date: string; example: string; index: number; total: number } {
  if (examples.length === 0) {
    throw new Error("No Turborepo examples are available for maintenance.");
  }
  const sortedExamples = [...examples].sort();
  const dayNumber = Math.floor(date.getTime() / MILLISECONDS_PER_DAY);
  const index = dayNumber % sortedExamples.length;
  return {
    date: date.toISOString().slice(0, 10),
    example: sortedExamples[index] as string,
    index,
    total: sortedExamples.length
  };
}

export async function resolveAutomatedExample(
  sandbox: SandboxSession,
  auth: AppPrincipal | null | undefined,
  sessionId: string,
  requestedExample?: string
): Promise<string | undefined> {
  if (!isAppPrincipal(auth)) return requestedExample;

  const selected = (await resolveAutomatedSelection(sandbox, auth, sessionId))
    .example;
  if (requestedExample && requestedExample !== selected) {
    throw new Error(
      `Today's automated maintenance target is '${selected}', not '${requestedExample}'.`
    );
  }
  return selected;
}

export async function resolveAutomatedSelection(
  sandbox: SandboxSession,
  auth: AppPrincipal,
  sessionId: string
): Promise<{ date: string; example: string; index: number; total: number }> {
  const examples = await listExampleNames(sandbox);
  const automatic = selectDailyExample(examples, sessionDate(sessionId));
  const requested = auth.attributes?.maintenanceExample;
  if (typeof requested !== "string") return automatic;

  const index = examples.indexOf(requested);
  if (index === -1) throw new Error(`Unknown example: ${requested}`);
  return { ...automatic, example: requested, index };
}

export function sessionDate(sessionId: string): Date {
  const encoded = sessionId.startsWith("wrun_")
    ? sessionId.slice(5, 15)
    : sessionId.slice(0, 10);
  if (encoded.length !== 10) {
    throw new Error(`Invalid Eve session id: ${sessionId}`);
  }

  let timestamp = 0;
  for (const character of encoded) {
    const value = ULID_ALPHABET.indexOf(character.toUpperCase());
    if (value === -1) throw new Error(`Invalid Eve session id: ${sessionId}`);
    timestamp = timestamp * 32 + value;
  }
  return new Date(timestamp);
}

export function isAppPrincipal(
  auth: AppPrincipal | null | undefined
): auth is AppPrincipal {
  return (
    auth?.authenticator === "app" &&
    auth.principalId === "eve:app" &&
    auth.principalType === "runtime"
  );
}

export async function getExamplePath(
  sandbox: SandboxSession,
  example: string
): Promise<string> {
  if (example.includes("/") || example.includes("\\") || example === "..") {
    throw new Error(`Invalid example name: ${example}`);
  }

  const examples = await listExampleNames(sandbox);
  if (!examples.includes(example)) {
    throw new Error(`Unknown example: ${example}`);
  }

  return path.join(await getRepoRoot(sandbox), "examples", example);
}

export async function readJsonFile(
  sandbox: SandboxSession,
  filePath: string
): Promise<JsonObject> {
  const content = await sandbox.readTextFile({ path: filePath });
  if (content === null) {
    throw new Error(`${filePath} does not exist.`);
  }
  const value: unknown = JSON.parse(content);
  if (!isJsonObject(value)) {
    throw new Error(`${filePath} must contain a JSON object.`);
  }
  return value;
}

export async function readTextIfExists(
  sandbox: SandboxSession,
  filePath: string,
  maxLines = 120
): Promise<string | null> {
  return sandbox.readTextFile({
    path: filePath,
    startLine: 1,
    endLine: maxLines
  });
}

export async function findPackageJsonFiles(
  sandbox: SandboxSession,
  root: string
): Promise<string[]> {
  const repoRoot = await getRepoRoot(sandbox);
  const relativeRoot = path.relative(repoRoot, root);
  const files = await listTrackedFiles(sandbox, relativeRoot);
  return files
    .filter((file) => path.basename(file) === "package.json")
    .map((file) => path.join(repoRoot, file));
}

export async function listTrackedFiles(
  sandbox: SandboxSession,
  relativeRoot: string
): Promise<string[]> {
  const repoRoot = await getRepoRoot(sandbox);
  const result = await sandbox.run({
    command: `git ls-files --cached --others --exclude-standard -- ${shellQuote(relativeRoot)}`,
    workingDirectory: repoRoot
  });
  assertCommandSucceeded(result, "List tracked repository files");
  const deleted = await sandbox.run({
    command: `git ls-files --deleted -- ${shellQuote(relativeRoot)}`,
    workingDirectory: repoRoot
  });
  assertCommandSucceeded(deleted, "List deleted repository files");
  const deletedFiles = new Set(deleted.stdout.split("\n").filter(Boolean));
  return result.stdout
    .split("\n")
    .filter((file) => file !== "" && !deletedFiles.has(file))
    .sort();
}

export async function listDirectory(
  sandbox: SandboxSession,
  directory: string
): Promise<DirectoryEntry[]> {
  const result = await sandbox.run({
    command: "find . -mindepth 1 -maxdepth 1 -printf '%f\\t%y\\n'",
    workingDirectory: directory
  });
  assertCommandSucceeded(result, `List ${directory}`);
  return result.stdout
    .split("\n")
    .filter(Boolean)
    .map((line) => {
      const [name = "", type = ""] = line.split("\t");
      return {
        name,
        type: type === "d" ? "directory" : type === "f" ? "file" : "other"
      };
    });
}

export async function fileExists(
  sandbox: SandboxSession,
  filePath: string
): Promise<boolean> {
  const result = await sandbox.run({
    command: `test -f ${shellQuote(filePath)}`
  });
  return result.exitCode === 0;
}

export function packageManagerName(
  packageManager: unknown
): "bun" | "npm" | "pnpm" | "yarn" | null {
  if (typeof packageManager !== "string") {
    return null;
  }
  const name = packageManager.split("@")[0];
  return name === "bun" || name === "npm" || name === "pnpm" || name === "yarn"
    ? name
    : null;
}

export async function resolveExamplesFile(
  sandbox: SandboxSession,
  relativePath: string
): Promise<string> {
  const repoRoot = await getRepoRoot(sandbox);
  if (path.isAbsolute(relativePath)) {
    throw new Error("Use a repository-relative path under examples/.");
  }

  const normalized = path.normalize(relativePath);
  if (normalized === ".." || normalized.startsWith("../")) {
    throw new Error("Path must stay inside the repository.");
  }
  if (normalized !== "examples" && !normalized.startsWith("examples/")) {
    throw new Error(
      "Only files under examples/ can be modified by this agent."
    );
  }

  return path.join(repoRoot, normalized);
}

export async function writeExamplesFile(
  sandbox: SandboxSession,
  relativePath: string,
  content: string
): Promise<{ path: string; bytes: number }> {
  const filePath = await resolveExamplesFile(sandbox, relativePath);
  if (lockfileNames.has(path.basename(filePath))) {
    throw new Error(
      "Lockfiles must be updated by running the package manager, not by direct writes."
    );
  }

  await sandbox.writeTextFile({ path: filePath, content });
  return {
    path: path.relative(await getRepoRoot(sandbox), filePath),
    bytes: Buffer.byteLength(content)
  };
}

export async function runCommand(
  sandbox: SandboxSession,
  command: string,
  args: string[],
  cwd: string,
  timeoutMs: number
): Promise<CommandResult> {
  const timeoutSeconds = Math.max(1, Math.ceil(timeoutMs / 1_000));
  const commandLine = [command, ...args].map(shellQuote).join(" ");
  const result = await sandbox.run({
    command: `timeout --signal=TERM ${timeoutSeconds}s ${commandLine}`,
    workingDirectory: cwd
  });
  return {
    command: commandLine,
    cwd,
    exitCode: result.exitCode,
    timedOut: result.exitCode === 124,
    stdout: truncateOutput(result.stdout),
    stderr: truncateOutput(result.stderr)
  };
}

export async function runCommandOrThrow(
  sandbox: SandboxSession,
  command: string,
  args: string[],
  cwd: string,
  timeoutMs: number
): Promise<CommandResult> {
  const result = await runCommand(sandbox, command, args, cwd, timeoutMs);
  if (result.exitCode === 0) {
    return result;
  }

  const output = [result.stdout, result.stderr].filter(Boolean).join("\n");
  const reason = result.timedOut
    ? "timed out"
    : `failed with exit code ${result.exitCode}`;
  throw new Error(
    `Command ${result.command} ${reason} in ${result.cwd}${output ? `:\n${output}` : "."}`
  );
}

export function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function pickJsonObject(value: unknown): JsonObject | null {
  return isJsonObject(value) ? value : null;
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}

function assertCommandSucceeded(
  result: { exitCode: number; stderr: string },
  operation: string
): void {
  if (result.exitCode !== 0) {
    throw new Error(`${operation} failed: ${result.stderr}`);
  }
}

function truncateOutput(output: string): string {
  if (output.length <= MAX_OUTPUT_LENGTH) {
    return output;
  }
  return `[output truncated]\n${output.slice(-MAX_OUTPUT_LENGTH)}`;
}
