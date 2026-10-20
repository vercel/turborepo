import { createHash } from "node:crypto";

import type { SandboxSession } from "eve/sandbox";

const checkout = "turborepo";
const stateDirectory = ".example-validation";

interface ValidationState {
  example: string;
  fingerprint: string;
  status: "failed" | "pending" | "success";
  tasks: string[];
}

export async function exampleChangeFingerprint(
  sandbox: SandboxSession,
  example: string
): Promise<string> {
  const scope = `examples/${example}`;
  const [head, tracked, deleted, untracked] = await Promise.all([
    runGit(sandbox, "git rev-parse HEAD"),
    runGit(
      sandbox,
      `git diff --no-renames --name-only --diff-filter=ACMRTUXB HEAD -- ${shellQuote(scope)}`
    ),
    runGit(
      sandbox,
      `git diff --no-renames --name-only --diff-filter=D HEAD -- ${shellQuote(scope)}`
    ),
    runGit(
      sandbox,
      `git ls-files --others --exclude-standard -- ${shellQuote(scope)}`
    )
  ]);
  const deletedPaths = new Set(lines(deleted));
  const paths = [
    ...new Set([...lines(tracked), ...lines(deleted), ...lines(untracked)])
  ].sort();
  const hash = createHash("sha256").update(`HEAD\0${head.trim()}\0`);

  for (const file of paths) {
    hash.update(`path\0${file}\0`);
    if (deletedPaths.has(file)) {
      hash.update("deleted\0");
      continue;
    }
    const content = await sandbox.readBinaryFile({
      path: `${checkout}/${file}`
    });
    if (content === null) {
      throw new Error(`Changed file ${file} disappeared while validating.`);
    }
    const executable = await sandbox.run({
      command: `test -x ${shellQuote(file)}`,
      workingDirectory: checkout
    });
    hash.update(executable.exitCode === 0 ? "100755\0" : "100644\0");
    hash.update(content);
    hash.update("\0");
  }

  return hash.digest("hex");
}

export async function writeValidationState(
  sandbox: SandboxSession,
  sessionId: string,
  state: ValidationState
): Promise<void> {
  const directory = await sandbox.run({
    command: `mkdir -p ${stateDirectory}`
  });
  if (directory.exitCode !== 0) {
    throw new Error(`Unable to create validation sidecar: ${directory.stderr}`);
  }
  await sandbox.writeTextFile({
    path: statePath(sessionId),
    content: `${JSON.stringify(state)}\n`
  });
}

export async function requireSuccessfulValidation(
  sandbox: SandboxSession,
  sessionId: string,
  example: string
): Promise<void> {
  const content = await sandbox.readTextFile({ path: statePath(sessionId) });
  if (content === null) {
    throw new Error(
      `Automated maintenance for ${example} requires successful Turbo validation before publishing.`
    );
  }

  let state: unknown;
  try {
    state = JSON.parse(content);
  } catch {
    throw new Error(`Turbo validation state for ${example} is invalid.`);
  }
  if (!isSuccessfulState(state) || state.example !== example) {
    throw new Error(
      `Automated maintenance for ${example} does not have successful Turbo validation.`
    );
  }

  const fingerprint = await exampleChangeFingerprint(sandbox, example);
  if (state.fingerprint !== fingerprint) {
    throw new Error(
      `Turbo validation for ${example} is stale because the example changed after validation.`
    );
  }
}

function isSuccessfulState(value: unknown): value is ValidationState {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const state = value as Partial<ValidationState>;
  return (
    state.status === "success" &&
    typeof state.example === "string" &&
    typeof state.fingerprint === "string" &&
    Array.isArray(state.tasks) &&
    state.tasks.every((task) => typeof task === "string")
  );
}

async function runGit(
  sandbox: SandboxSession,
  command: string
): Promise<string> {
  const result = await sandbox.run({ command, workingDirectory: checkout });
  if (result.exitCode !== 0) {
    throw new Error(`${command} failed: ${result.stderr}`);
  }
  return result.stdout;
}

function lines(output: string): string[] {
  return output.split("\n").filter(Boolean);
}

function statePath(sessionId: string): string {
  const key = createHash("sha256").update(sessionId).digest("hex");
  return `${stateDirectory}/${key}.json`;
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}
