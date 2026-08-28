import { randomUUID } from "node:crypto";

import { getVercelOidcToken } from "@vercel/oidc";
import { APIError, Sandbox } from "@vercel/sandbox";

import { FACTORY_IMAGE_SPEC } from "./factory-image";
import {
  refreshFactoryCheckout,
  requireFactoryImage
} from "./current-factory-image";
import { readFactoryImagePointer } from "./factory-image-registry";
import {
  FX_TERMINAL_RUNNER_PATH,
  FX_TERMINAL_RUNNER_SOURCE,
  FX_TERMINAL_SESSION_PATH,
  FX_TERMINAL_TMUX_SESSION,
  parseFxTerminalResult
} from "./fx-terminal-runner";
import { fxEnvironment } from "./fx-environment";
import type { FxTurnResult } from "./fx-result";
import { getGitHubToken } from "./github";
import {
  applyWorkspaceNetworkPolicy,
  workspaceNetworkPolicy
} from "./workspace-network-policy";
import {
  installWorkspacePublishCommand,
  type WorkspacePublishBridge
} from "./workspace-publish";

const WORKSPACE_TIMEOUT_MS = 45 * 60 * 1000;
const WORKSPACE_VCPUS = 8;

export async function getFxWorkspaceSandbox(
  name: string,
  publishBridge?: WorkspacePublishBridge | null
): Promise<Sandbox> {
  const sandbox = await Sandbox.get({ name, resume: true });
  await applyWorkspaceNetworkPolicy(
    sandbox,
    workspaceNetworkPolicy(await getGitHubToken(), publishBridge)
  );
  await installWorkspacePublishCommand(sandbox, publishBridge ?? null);
  return sandbox;
}

export async function getOrCreateFxWorkspaceSandbox(
  name: string,
  publishBridge?: WorkspacePublishBridge | null
): Promise<Sandbox> {
  try {
    return await getFxWorkspaceSandbox(name, publishBridge);
  } catch (error) {
    if (!isMissing(error)) throw error;
  }

  const [githubToken, pointer] = await Promise.all([
    getGitHubToken(),
    readFactoryImagePointer()
  ]);
  const image = requireFactoryImage(pointer);
  const shared = {
    env: {
      FX_AUTO_UPGRADE: "0",
      FX_PERMISSION_MODE: "yolo",
      GH_TOKEN: "brokered-by-sandbox"
    },
    name,
    networkPolicy: "deny-all" as const,
    resources: { vcpus: WORKSPACE_VCPUS },
    tags: { role: "factory-workspace" },
    timeout: WORKSPACE_TIMEOUT_MS
  };

  let sandbox: Sandbox;
  try {
    sandbox = await Sandbox.create({
      ...shared,
      source: { snapshotId: image.snapshotId, type: "snapshot" }
    });
  } catch (error) {
    if (!isConflict(error)) throw error;
    return Sandbox.get({ name });
  }

  await applyWorkspaceNetworkPolicy(
    sandbox,
    workspaceNetworkPolicy(githubToken, publishBridge)
  );
  await refreshFactoryCheckout(sandbox, FACTORY_IMAGE_SPEC.checkoutPath);
  await installWorkspacePublishCommand(sandbox, publishBridge ?? null);
  return sandbox;
}

export async function runFxTurn(
  sandbox: Sandbox,
  prompt: string,
  sessionId?: string,
  getOidcToken: () => Promise<string> = getVercelOidcToken,
  onSession?: (sessionId: string) => Promise<void>
): Promise<FxTurnResult & { readonly cancelled: boolean }> {
  const promptPath = `/factory/state/fx-terminal-prompt-${randomUUID()}`;
  const tokenPath = `/factory/state/fx-terminal-oidc-${randomUUID()}`;
  const oidcToken = await getOidcToken();
  await sandbox.runCommand({
    args: ["-f", FX_TERMINAL_SESSION_PATH],
    cmd: "rm",
    timeoutMs: 10_000
  });
  await sandbox.writeFiles([
    {
      content: Buffer.from(FX_TERMINAL_RUNNER_SOURCE, "utf8"),
      path: FX_TERMINAL_RUNNER_PATH
    },
    { content: Buffer.from(prompt, "utf8"), path: promptPath },
    { content: Buffer.from(oidcToken, "utf8"), path: tokenPath }
  ]);
  const command = await sandbox.runCommand({
    args: [
      FX_TERMINAL_RUNNER_PATH,
      FACTORY_IMAGE_SPEC.checkoutPath,
      promptPath,
      tokenPath,
      sessionId ?? "",
      FX_TERMINAL_SESSION_PATH,
      FX_TERMINAL_TMUX_SESSION
    ],
    cmd: "node",
    detached: true,
    env: fxEnvironment(oidcToken),
    timeoutMs: WORKSPACE_TIMEOUT_MS - 60_000
  });

  if (!sessionId && onSession) {
    const createdSessionId = await waitForTerminalSession(sandbox);
    await onSession(createdSessionId);
  }

  const finished = await command.wait();
  const parsed = parseFxTerminalResult(await finished.stdout());
  if (finished.exitCode !== 0 || parsed === null) {
    const stderr = (await finished.stderr()).slice(0, 2000);
    throw new Error(stderr || "fx did not complete the workspace turn.");
  }
  return { ...parsed, cancelled: false };
}

async function waitForTerminalSession(sandbox: Sandbox): Promise<string> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const command = await sandbox.runCommand({
      args: ["-lc", `cat ${FX_TERMINAL_SESSION_PATH} 2>/dev/null || true`],
      cmd: "bash",
      timeoutMs: 10_000
    });
    const sessionId = (await command.stdout()).trim();
    if (sessionId) return sessionId;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("fx did not create an interactive session.");
}

function isMissing(error: unknown): boolean {
  return error instanceof APIError && error.response.status === 404;
}

function isConflict(error: unknown): boolean {
  return error instanceof APIError && error.response.status === 409;
}
