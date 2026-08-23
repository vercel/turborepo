import { timingSafeEqual } from "node:crypto";

import type { Sandbox } from "@vercel/sandbox";
import type { SandboxSession } from "eve/sandbox";

import type { WorkspaceRecord } from "./workspace";

const CHECKOUT_PATH = "/factory/turborepo";
const FX_INSTRUCTIONS_PATH = "/home/vercel/.fx/AGENTS.md";
const PUBLISH_COMMAND_PATH = "/factory/bin/factory-create-pr";
const PUBLISH_INSTRUCTIONS = `When the maintainer asks to create or make a pull request, leave repository changes uncommitted and run \`factory-create-pr --branch agents/<topic> --title "type: Description" --body "validation results"\`. Factory creates the verified commit, branch, and draft pull request through Eve's GitHub credentials. Never run git commit, git push, gh auth setup-git, or gh pr create.\n`;

export interface WorkspacePublishBridge {
  readonly authorization: string;
  readonly hostname: string;
  readonly path: string;
  readonly url: string;
}

export interface WorkspacePublishInput {
  readonly body: string;
  readonly branchName: string;
  readonly title: string;
}

export function workspacePublishBridge(
  workspaceId: string,
  publishToken: string
): WorkspacePublishBridge | null {
  const rawHostname =
    process.env.VERCEL_URL ?? process.env.VERCEL_PROJECT_PRODUCTION_URL;
  if (!rawHostname) return null;
  const hostname = rawHostname.replace(/^https?:\/\//, "").replace(/\/$/, "");
  const path = `/api/workspaces/${workspaceId}/publish`;
  return {
    authorization: `Bearer ${publishToken}`,
    hostname,
    path,
    url: `https://${hostname}${path}`
  };
}

export async function installWorkspacePublishCommand(
  sandbox: Sandbox,
  bridge: WorkspacePublishBridge | null
): Promise<void> {
  if (bridge === null) return;
  const source = `#!/usr/bin/env node
const values = process.argv.slice(2);
const input = {};
for (let index = 0; index < values.length; index += 2) {
  const key = values[index];
  const value = values[index + 1];
  if (!key?.startsWith("--") || value === undefined) {
    console.error("Usage: factory-create-pr --branch agents/<topic> --title 'type: Description' --body 'Validation results'");
    process.exit(2);
  }
  input[key.slice(2)] = value;
}
const response = await fetch(${JSON.stringify(bridge.url)}, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ branchName: input.branch, title: input.title, body: input.body ?? "" })
});
const text = await response.text();
if (!response.ok) {
  console.error(text || "Factory could not create the pull request.");
  process.exit(1);
}
try {
  const result = JSON.parse(text);
  console.log(result.url ?? result.reason ?? text);
} catch {
  console.log(text);
}
`;
  await sandbox.runCommand({
    args: ["-p", "/factory/bin", "/home/vercel/.fx"],
    cmd: "mkdir",
    timeoutMs: 10_000
  });
  await sandbox.writeFiles([
    { content: Buffer.from(source, "utf8"), path: PUBLISH_COMMAND_PATH },
    {
      content: Buffer.from(PUBLISH_INSTRUCTIONS, "utf8"),
      path: FX_INSTRUCTIONS_PATH
    }
  ]);
  const command = await sandbox.runCommand({
    args: ["+x", PUBLISH_COMMAND_PATH],
    cmd: "chmod",
    timeoutMs: 10_000
  });
  if (command.exitCode !== 0)
    throw new Error(
      `Could not install Factory PR helper: ${await command.stderr()}`
    );
}

export function isWorkspacePublishRequest(
  request: Request,
  workspace: WorkspaceRecord
): boolean {
  const expected = workspace.publishToken;
  const actual = request.headers.get("authorization")?.replace(/^Bearer /, "");
  if (!expected || !actual || expected.length !== actual.length) return false;
  return timingSafeEqual(Buffer.from(expected), Buffer.from(actual));
}

export function parseWorkspacePublishInput(
  value: unknown
): WorkspacePublishInput | null {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    return null;
  const input = value as Record<string, unknown>;
  if (
    typeof input.branchName !== "string" ||
    !/^agents\/[A-Za-z0-9._/-]+$/.test(input.branchName) ||
    typeof input.title !== "string" ||
    !/^[a-z]+: [A-Z].+$/.test(input.title) ||
    typeof input.body !== "string" ||
    input.body.length > 50_000
  )
    return null;
  return {
    body: input.body,
    branchName: input.branchName,
    title: input.title
  };
}

export async function publishWorkspacePullRequest(
  sandbox: Sandbox,
  workspace: WorkspaceRecord,
  input: WorkspacePublishInput
) {
  const { createPullRequest } = await import("./create-pull-request");
  return createPullRequest(input, {
    auth: null,
    sandbox: sandboxSession(sandbox),
    sessionId: workspace.sessionId ?? workspace.id
  });
}

function sandboxSession(sandbox: Sandbox): SandboxSession {
  const checkout = CHECKOUT_PATH;
  const resolve = (path: string) =>
    path === "turborepo"
      ? checkout
      : path.startsWith("turborepo/")
        ? `${checkout}/${path.slice("turborepo/".length)}`
        : path;
  return {
    id: sandbox.name,
    resolvePath: resolve,
    async run(options: Parameters<SandboxSession["run"]>[0]) {
      const command = await sandbox.runCommand({
        args: ["-lc", options.command],
        cmd: "bash",
        cwd: options.workingDirectory
          ? resolve(options.workingDirectory)
          : checkout,
        timeoutMs: 10 * 60 * 1000
      });
      return {
        exitCode: command.exitCode,
        stderr: await command.stderr(),
        stdout: await command.stdout()
      };
    },
    async readBinaryFile(
      options: Parameters<SandboxSession["readBinaryFile"]>[0]
    ) {
      return sandbox.fs.readFile(resolve(options.path)).catch(() => null);
    }
  } as unknown as SandboxSession;
}
