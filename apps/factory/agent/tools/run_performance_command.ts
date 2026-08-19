import type { SandboxSession } from "eve/sandbox";
import { defineTool } from "eve/tools";
import { z } from "zod";

import {
  type PerformanceInput,
  recordCommandEvidence,
  repositoryChangeFingerprint
} from "../lib/performance-validation.js";
import { getRepoRoot, runCommand } from "../lib/repo.js";

export default defineTool({
  description:
    "Run one reproducible benchmark or correctness command in the Turborepo checkout and record its evidence for the current performance run.",
  inputSchema: z.object({
    command: z
      .string()
      .min(1)
      .regex(/^[A-Za-z0-9._+-]+$/),
    args: z.array(z.string()).max(30).default([]),
    fingerprintFiles: z.array(z.string().min(1)).max(2).default([]),
    fingerprintRepositories: z.array(z.string().min(1)).max(10).default([]),
    phase: z.enum(["baseline", "after", "comparison", "validation"]),
    timeoutSeconds: z.number().int().positive().max(1800).default(600)
  }),
  async execute(
    {
      command,
      args,
      fingerprintFiles,
      fingerprintRepositories,
      phase,
      timeoutSeconds
    },
    ctx
  ) {
    const sandbox = await ctx.getSandbox();
    const repoRoot = await getRepoRoot(sandbox);
    const inputs = await fingerprintInputs(
      sandbox,
      repoRoot,
      fingerprintFiles,
      fingerprintRepositories
    );
    const evidence = await runCommand(
      sandbox,
      command,
      args,
      repoRoot,
      timeoutSeconds * 1_000
    );
    const evidenceWithInputs = { ...evidence, inputs };
    await recordCommandEvidence(
      sandbox,
      ctx.session.id,
      phase,
      evidenceWithInputs
    );
    return evidenceWithInputs;
  }
});

async function fingerprintInputs(
  sandbox: SandboxSession,
  repoRoot: string,
  files: string[],
  repositories: string[]
): Promise<PerformanceInput[]> {
  const inputs: PerformanceInput[] = [];
  for (const file of files) {
    const result = await runCommand(
      sandbox,
      "sha256sum",
      [file],
      repoRoot,
      30_000
    );
    if (result.exitCode !== 0) {
      throw new Error(`Failed to fingerprint ${file}: ${result.stderr}`);
    }
    inputs.push({
      fingerprint: result.stdout.trim().split(/\s+/, 1)[0] as string,
      kind: "file",
      path: file
    });
  }
  for (const repository of repositories) {
    if (repository === "." || repository === repoRoot) {
      inputs.push({
        fingerprint: await repositoryChangeFingerprint(sandbox),
        kind: "repository",
        path: repository
      });
      continue;
    }
    const diff = await runCommand(
      sandbox,
      "git",
      ["-C", repository, "status", "--porcelain", "--untracked-files=all"],
      repoRoot,
      30_000
    );
    if (diff.exitCode !== 0 || diff.stdout.trim() !== "") {
      throw new Error(
        `Benchmark repository ${repository} has checkout changes.`
      );
    }
    const head = await runCommand(
      sandbox,
      "git",
      ["-C", repository, "rev-parse", "HEAD"],
      repoRoot,
      30_000
    );
    if (head.exitCode !== 0) {
      throw new Error(`Failed to fingerprint ${repository}: ${head.stderr}`);
    }
    inputs.push({
      fingerprint: head.stdout.trim(),
      kind: "repository",
      path: repository
    });
  }
  return inputs;
}
