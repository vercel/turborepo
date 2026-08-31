import { Sandbox } from "@vercel/sandbox";

const CHECKOUT_PATH = "/factory/turborepo";
const DIFF_SCRIPT = `set -euo pipefail
cd ${CHECKOUT_PATH}
git diff --no-ext-diff --no-color --find-renames HEAD --
while IFS= read -r -d '' path; do
  git diff --no-ext-diff --no-color --no-index -- /dev/null "$path" || [ "$?" -eq 1 ]
done < <(git ls-files --others --exclude-standard -z)`;
export const MAX_WORKSPACE_DIFF_BYTES = 4 * 1024 * 1024;

interface DiffCommand {
  readonly exitCode: number;
  stderr(): Promise<string>;
  stdout(): Promise<string>;
}

interface DiffSandbox {
  runCommand(
    command: string,
    args: string[],
    options: { readonly timeoutMs: number }
  ): Promise<DiffCommand>;
}

export class WorkspaceDiffTooLargeError extends Error {}

export async function readWorkspaceDiff(
  sandboxName: string,
  getSandbox: (name: string) => Promise<DiffSandbox> = async (name) =>
    Sandbox.get({ name, resume: true })
): Promise<string> {
  const sandbox = await getSandbox(sandboxName);
  const result = await sandbox.runCommand("bash", ["-lc", DIFF_SCRIPT], {
    timeoutMs: 30_000
  });

  if (result.exitCode !== 0) {
    const detail = (await result.stderr()).trim();
    throw new Error(detail || "Could not read the workspace git diff.");
  }

  const patch = await result.stdout();
  if (Buffer.byteLength(patch, "utf8") > MAX_WORKSPACE_DIFF_BYTES) {
    throw new WorkspaceDiffTooLargeError(
      "The workspace diff is too large to display."
    );
  }
  return patch;
}
