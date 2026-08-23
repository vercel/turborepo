import { getVercelOidcToken } from "@vercel/oidc";
import { APIError, Sandbox } from "@vercel/sandbox";

import {
  FACTORY_IMAGE_BASE,
  FACTORY_IMAGE_SPEC,
  factoryImageFingerprint,
  runFactoryImagePhases
} from "./factory-image";
import { readFactoryImagePointer } from "./factory-image-registry";
import {
  FX_ACP_CANCEL_PATH,
  FX_ACP_CLIENT_PATH,
  FX_ACP_CLIENT_SOURCE,
  FX_ACP_SESSION_PATH,
  parseFxAcpResult
} from "./fx-acp";
import { fxEnvironment } from "./fx-environment";
import type { FxTurnResult } from "./fx-result";
import { getGitHubToken } from "./github";
import { workspaceNetworkPolicy } from "./workspace-network-policy";

const WORKSPACE_TIMEOUT_MS = 45 * 60 * 1000;
const WORKSPACE_VCPUS = 8;

export async function getFxWorkspaceSandbox(name: string): Promise<Sandbox> {
  const sandbox = await Sandbox.get({ name, resume: true });
  await sandbox.updateNetworkPolicy(
    workspaceNetworkPolicy(await getGitHubToken())
  );
  return sandbox;
}

export async function getOrCreateFxWorkspaceSandbox(
  name: string
): Promise<Sandbox> {
  try {
    const sandbox = await getFxWorkspaceSandbox(name);
    if (!(await hasWorkspaceCheckout(sandbox))) {
      await initializeWorkspaceSandbox(sandbox, false);
    }
    return sandbox;
  } catch (error) {
    if (!isMissing(error)) throw error;
  }

  const [githubToken, pointer] = await Promise.all([
    getGitHubToken(),
    readFactoryImagePointer()
  ]);
  const image =
    pointer?.fingerprint === factoryImageFingerprint() ? pointer : null;
  const shared = {
    env: {
      FX_AUTO_UPGRADE: "0",
      FX_PERMISSION_MODE: "yolo",
      GH_TOKEN: "brokered-by-sandbox"
    },
    name,
    networkPolicy: workspaceNetworkPolicy(githubToken),
    resources: { vcpus: WORKSPACE_VCPUS },
    tags: { role: "factory-workspace" },
    timeout: WORKSPACE_TIMEOUT_MS
  };

  let sandbox: Sandbox;
  try {
    sandbox =
      image === null
        ? await Sandbox.create({ ...shared, image: FACTORY_IMAGE_BASE })
        : await Sandbox.create({
            ...shared,
            source: { snapshotId: image.snapshotId, type: "snapshot" }
          });
  } catch (error) {
    if (!isConflict(error)) throw error;
    return Sandbox.get({ name });
  }

  await initializeWorkspaceSandbox(sandbox, image !== null);
  return sandbox;
}

async function hasWorkspaceCheckout(sandbox: Sandbox): Promise<boolean> {
  try {
    const result = await sandbox.runCommand({
      args: ["-C", FACTORY_IMAGE_SPEC.checkoutPath, "rev-parse", "HEAD"],
      cmd: "git",
      timeoutMs: 30_000
    });
    return result.exitCode === 0;
  } catch {
    return false;
  }
}

async function initializeWorkspaceSandbox(
  sandbox: Sandbox,
  fromSnapshot: boolean
): Promise<void> {
  try {
    if (!fromSnapshot) {
      await runFactoryImagePhases(
        {
          async run(command) {
            const result = await sandbox.runCommand({
              args: ["-lc", command],
              cmd: "bash"
            });
            return {
              exitCode: result.exitCode,
              stderr: await result.stderr(),
              stdout: await result.stdout()
            };
          }
        },
        { revision: "main" }
      );
      return;
    }

    const result = await sandbox.runCommand({
      args: [
        "-lc",
        `git -C ${FACTORY_IMAGE_SPEC.checkoutPath} fetch --depth=1 --force origin main && git -C ${FACTORY_IMAGE_SPEC.checkoutPath} reset --hard FETCH_HEAD && git -C ${FACTORY_IMAGE_SPEC.checkoutPath} clean -ffd && cd ${FACTORY_IMAGE_SPEC.checkoutPath} && pnpm install --frozen-lockfile`
      ],
      cmd: "bash"
    });
    if (result.exitCode !== 0) {
      throw new Error(
        `Could not initialize fx workspace: ${await result.stderr()}`
      );
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    await sandbox
      .writeFiles([
        {
          content: Buffer.from(`${message}\n`, "utf8"),
          path: "/factory/state/error"
        }
      ])
      .catch(() => {});
    throw error;
  }
}

export async function runFxTurn(
  sandbox: Sandbox,
  prompt: string,
  sessionId?: string,
  getOidcToken: () => Promise<string> = getVercelOidcToken,
  onSession?: (sessionId: string) => Promise<void>
): Promise<FxTurnResult & { readonly cancelled: boolean }> {
  await sandbox.writeFiles([
    {
      content: Buffer.from(FX_ACP_CLIENT_SOURCE, "utf8"),
      path: FX_ACP_CLIENT_PATH
    }
  ]);
  await sandbox.runCommand({
    args: ["-f", FX_ACP_SESSION_PATH, FX_ACP_CANCEL_PATH],
    cmd: "rm",
    timeoutMs: 10_000
  });
  const command = await sandbox.runCommand({
    args: [
      FX_ACP_CLIENT_PATH,
      FACTORY_IMAGE_SPEC.checkoutPath,
      prompt,
      sessionId ?? "",
      FX_ACP_SESSION_PATH,
      FX_ACP_CANCEL_PATH
    ],
    cmd: "node",
    detached: true,
    env: fxEnvironment(await getOidcToken()),
    timeoutMs: WORKSPACE_TIMEOUT_MS - 60_000
  });

  if (!sessionId && onSession) {
    const createdSessionId = await waitForAcpSession(sandbox);
    await onSession(createdSessionId);
  }

  const finished = await command.wait();
  const parsed = parseFxAcpResult(await finished.stdout());
  if (finished.exitCode !== 0 || parsed === null) {
    const stderr = (await finished.stderr()).slice(0, 2000);
    throw new Error(stderr || "fx did not complete the workspace turn.");
  }
  return parsed;
}

async function waitForAcpSession(sandbox: Sandbox): Promise<string> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const command = await sandbox.runCommand({
      args: ["-lc", `cat ${FX_ACP_SESSION_PATH} 2>/dev/null || true`],
      cmd: "bash",
      timeoutMs: 10_000
    });
    const sessionId = (await command.stdout()).trim();
    if (sessionId) return sessionId;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("fx did not create an ACP session.");
}

function isMissing(error: unknown): boolean {
  return error instanceof APIError && error.response.status === 404;
}

function isConflict(error: unknown): boolean {
  return error instanceof APIError && error.response.status === 409;
}
