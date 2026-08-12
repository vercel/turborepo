import { createHash } from "node:crypto";

import type { SandboxSession } from "eve/sandbox";

import {
  type PerformanceModelSelection,
  type PerformanceReviewer,
  selectPerformanceModels
} from "./performance-models.js";
import { sessionDate } from "./repo.js";

const checkout = "turborepo";
const stateDirectory = ".performance-validation";

export interface CommandEvidence {
  command: string;
  exitCode: number;
  stderr: string;
  stdout: string;
}

export interface PerformanceState extends PerformanceModelSelection {
  date: string;
  baseline?: CommandEvidence;
  after?: CommandEvidence;
  validations: CommandEvidence[];
  validatedFingerprint?: string;
  review?: {
    approved: boolean;
    blockingFindings: string[];
    reviewer: PerformanceReviewer;
    summary: string;
  };
  reviewedFingerprint?: string;
}

export async function beginPerformanceRun(
  sandbox: SandboxSession,
  sessionId: string
): Promise<PerformanceState> {
  const date = sessionDate(sessionId);
  const state: PerformanceState = {
    date: date.toISOString().slice(0, 10),
    ...selectPerformanceModels(date),
    validations: []
  };
  await writeState(sandbox, sessionId, state);
  return state;
}

export async function readPerformanceState(
  sandbox: SandboxSession,
  sessionId: string
): Promise<PerformanceState | null> {
  const content = await sandbox.readTextFile({ path: statePath(sessionId) });
  if (content === null) return null;
  const value: unknown = JSON.parse(content);
  if (!isPerformanceState(value)) {
    throw new Error("Performance validation state is invalid.");
  }
  return value;
}

export async function recordCommandEvidence(
  sandbox: SandboxSession,
  sessionId: string,
  phase: "after" | "baseline" | "validation",
  evidence: CommandEvidence
): Promise<PerformanceState> {
  const state = await requireState(sandbox, sessionId);
  if (phase === "baseline") {
    if (await hasChanges(sandbox)) {
      throw new Error("Record the baseline before modifying the checkout.");
    }
    state.baseline = evidence;
  } else if (phase === "after") {
    if (!state.baseline) throw new Error("Run the baseline measurement first.");
    if (state.baseline.command !== evidence.command) {
      throw new Error(
        "The after measurement must use the exact baseline command."
      );
    }
    state.after = evidence;
    state.validations = [];
    state.review = undefined;
    state.validatedFingerprint = undefined;
    state.reviewedFingerprint = undefined;
  } else {
    if (!state.after) throw new Error("Run the after measurement first.");
    state.validations.push(evidence);
    state.validatedFingerprint = await repositoryChangeFingerprint(sandbox);
    state.review = undefined;
    state.reviewedFingerprint = undefined;
  }
  await writeState(sandbox, sessionId, state);
  return state;
}

export async function recordPerformanceReview(
  sandbox: SandboxSession,
  sessionId: string,
  review: PerformanceState["review"]
): Promise<PerformanceState> {
  const state = await requireState(sandbox, sessionId);
  await requireMeasurementsAndValidation(sandbox, state);
  if (!review || review.reviewer !== state.reviewer) {
    throw new Error(`Review must come from ${state.reviewer}.`);
  }
  if (review.approved && review.blockingFindings.length > 0) {
    throw new Error("An approved review cannot contain blocking findings.");
  }
  state.review = review;
  state.reviewedFingerprint = await repositoryChangeFingerprint(sandbox);
  await writeState(sandbox, sessionId, state);
  return state;
}

export async function requirePublishablePerformanceChange(
  sandbox: SandboxSession,
  sessionId: string
): Promise<PerformanceState> {
  const state = await requireState(sandbox, sessionId);
  await requireMeasurementsAndValidation(sandbox, state);
  if (!state.review?.approved || state.review.blockingFindings.length > 0) {
    throw new Error(
      "The opposite-model adversarial review has not approved this change."
    );
  }
  const fingerprint = await repositoryChangeFingerprint(sandbox);
  if (state.reviewedFingerprint !== fingerprint) {
    throw new Error(
      "The adversarial review is stale because the diff changed."
    );
  }
  return state;
}

export async function repositoryChangeFingerprint(
  sandbox: SandboxSession
): Promise<string> {
  const [head, tracked, deleted, untracked] = await Promise.all([
    runGit(sandbox, "git rev-parse HEAD"),
    runGit(
      sandbox,
      "git diff --no-renames --name-only --diff-filter=ACMRTUXB HEAD"
    ),
    runGit(sandbox, "git diff --no-renames --name-only --diff-filter=D HEAD"),
    runGit(sandbox, "git ls-files --others --exclude-standard")
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
    if (content === null) throw new Error(`Changed file ${file} disappeared.`);
    hash.update(content);
    hash.update("\0");
  }
  return hash.digest("hex");
}

async function requireMeasurementsAndValidation(
  sandbox: SandboxSession,
  state: PerformanceState
): Promise<void> {
  if (state.baseline?.exitCode !== 0 || state.after?.exitCode !== 0) {
    throw new Error("Successful baseline and after measurements are required.");
  }
  if (
    state.validations.length === 0 ||
    state.validations.some((item) => item.exitCode !== 0)
  ) {
    throw new Error(
      "At least one successful correctness validation is required."
    );
  }
  const fingerprint = await repositoryChangeFingerprint(sandbox);
  if (state.validatedFingerprint !== fingerprint) {
    throw new Error("Validation is stale because the diff changed.");
  }
}

async function requireState(sandbox: SandboxSession, sessionId: string) {
  const state = await readPerformanceState(sandbox, sessionId);
  if (!state) throw new Error("Call begin_performance_improvement first.");
  return state;
}

async function writeState(
  sandbox: SandboxSession,
  sessionId: string,
  state: PerformanceState
): Promise<void> {
  const directory = await sandbox.run({
    command: `mkdir -p ${stateDirectory}`
  });
  if (directory.exitCode !== 0) throw new Error(directory.stderr);
  await sandbox.writeTextFile({
    path: statePath(sessionId),
    content: `${JSON.stringify(state)}\n`
  });
}

async function hasChanges(sandbox: SandboxSession): Promise<boolean> {
  return (await runGit(sandbox, "git status --short")).trim() !== "";
}

async function runGit(
  sandbox: SandboxSession,
  command: string
): Promise<string> {
  const result = await sandbox.run({ command, workingDirectory: checkout });
  if (result.exitCode !== 0)
    throw new Error(`${command} failed: ${result.stderr}`);
  return result.stdout;
}

function lines(output: string): string[] {
  return output.split("\n").filter(Boolean);
}

function statePath(sessionId: string): string {
  return `${stateDirectory}/${createHash("sha256").update(sessionId).digest("hex")}.json`;
}

function isPerformanceState(value: unknown): value is PerformanceState {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    return false;
  const state = value as Partial<PerformanceState>;
  return (
    typeof state.date === "string" &&
    typeof state.authorModel === "string" &&
    typeof state.reviewerModel === "string" &&
    (state.reviewer === "fable_performance_reviewer" ||
      state.reviewer === "gpt_performance_reviewer") &&
    Array.isArray(state.validations)
  );
}
