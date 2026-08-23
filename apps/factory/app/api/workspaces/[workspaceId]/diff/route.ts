import { Sandbox } from "@vercel/sandbox";

import { getWorkspace } from "../../../../../agent/lib/workspace-store";
import { isSafeWorkspaceDiffPath } from "../../../../../agent/lib/workspace";

const MAX_OUTPUT_LENGTH = 100_000;
const MAX_UNTRACKED_FILE_BYTES = 1_000_000;
const MAX_UNTRACKED_FILES = 50;
const REPOSITORY_DIRECTORIES = ["turborepo", "."] as const;

export async function GET(
  _request: Request,
  context: { params: Promise<{ workspaceId: string }> }
): Promise<Response> {
  const { workspaceId } = await context.params;
  const workspace = await getWorkspace(workspaceId);
  if (!workspace)
    return Response.json({ error: "Workspace not found." }, { status: 404 });

  const sandbox = await Sandbox.get({ name: workspace.sandbox.name });
  const status = await runGit(sandbox, ["status", "--short"]);
  const tracked = await runGit(sandbox, [
    "diff",
    "--no-ext-diff",
    "HEAD",
    "--"
  ]);
  const untracked = (
    await runGit(sandbox, ["ls-files", "--others", "--exclude-standard", "-z"])
  )
    .split("\0")
    .filter((path) => path && isSafeWorkspaceDiffPath(path))
    .slice(0, MAX_UNTRACKED_FILES);
  const cwd = await findRepositoryDirectory(sandbox);
  const untrackedDiffs: string[] = [];
  for (const path of untracked) {
    const stat = await sandbox.fs.stat(`${cwd}/${path}`).catch(() => null);
    if (stat === null || stat.size > MAX_UNTRACKED_FILE_BYTES) continue;
    untrackedDiffs.push(
      await runGit(
        sandbox,
        ["diff", "--no-index", "--", "/dev/null", path],
        [0, 1]
      )
    );
  }
  const diff = [tracked, ...untrackedDiffs].filter(Boolean).join("\n");
  return Response.json(
    {
      diff: diff.slice(0, MAX_OUTPUT_LENGTH),
      status: status.slice(0, MAX_OUTPUT_LENGTH)
    },
    { headers: { "cache-control": "no-store" } }
  );
}

async function runGit(
  sandbox: Sandbox,
  args: readonly string[],
  allowedExitCodes: readonly number[] = [0]
): Promise<string> {
  for (const cwd of REPOSITORY_DIRECTORIES) {
    const result = await sandbox.runCommand({
      args: [...args],
      cmd: "git",
      cwd,
      timeoutMs: 30_000
    });
    if (allowedExitCodes.includes(result.exitCode)) return result.stdout();
  }
  throw new Error("Workspace repository is unavailable.");
}

async function findRepositoryDirectory(sandbox: Sandbox): Promise<string> {
  for (const cwd of REPOSITORY_DIRECTORIES) {
    const result = await sandbox.runCommand({
      args: ["rev-parse", "--show-toplevel"],
      cmd: "git",
      cwd,
      timeoutMs: 30_000
    });
    if (result.exitCode === 0) return cwd;
  }
  throw new Error("Workspace repository is unavailable.");
}
