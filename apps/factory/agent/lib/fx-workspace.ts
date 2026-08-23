import { APIError, type NetworkPolicy, Sandbox } from "@vercel/sandbox";

import {
  FACTORY_IMAGE_BASE,
  FACTORY_IMAGE_SPEC,
  factoryImageFingerprint,
  runFactoryImagePhases
} from "./factory-image";
import { readFactoryImagePointer } from "./factory-image-registry";
import { parseFxTurnResult, type FxTurnResult } from "./fx-result.js";
import { getGitHubToken } from "./github";

const WORKSPACE_TIMEOUT_MS = 45 * 60 * 1000;
const WORKSPACE_VCPUS = 8;

export async function getOrCreateFxWorkspaceSandbox(
  name: string
): Promise<Sandbox> {
  try {
    const sandbox = await Sandbox.get({ name });
    await sandbox.updateNetworkPolicy(
      workspaceNetworkPolicy(await getGitHubToken())
    );
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

  if (image === null) {
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
  } else {
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
  }
  return sandbox;
}

function workspaceNetworkPolicy(githubToken: string): NetworkPolicy {
  return {
    allow: {
      "*": [],
      "api.github.com": [
        {
          match: {
            method: ["GET", "POST", "PATCH"],
            path: { startsWith: "/repos/vercel/turborepo" }
          },
          transform: [{ headers: { authorization: `Bearer ${githubToken}` } }]
        }
      ],
      "github.com": [
        {
          match: {
            method: ["GET", "POST"],
            path: { startsWith: "/vercel/turborepo.git" }
          },
          transform: [{ headers: { authorization: `Bearer ${githubToken}` } }]
        }
      ]
    },
    subnets: { deny: ["169.254.169.254/32"] }
  };
}

export async function runFxTurn(
  sandbox: Sandbox,
  prompt: string,
  sessionId?: string
): Promise<FxTurnResult> {
  const args = ["ask", "--json", "--yolo"];
  if (sessionId) args.push("--resume-id", sessionId);
  args.push("--", prompt);
  const command = await sandbox.runCommand({
    args,
    cmd: "fx",
    cwd: FACTORY_IMAGE_SPEC.checkoutPath,
    env: { FX_AUTO_UPGRADE: "0", FX_PERMISSION_MODE: "yolo" },
    timeoutMs: WORKSPACE_TIMEOUT_MS - 60_000
  });
  const stdout = await command.stdout();
  const parsed = parseFxTurnResult(stdout, command.exitCode);
  if (parsed === null) {
    const stderr = (await command.stderr()).slice(0, 2000);
    throw new Error(stderr || "fx did not complete the workspace turn.");
  }
  return parsed;
}

function isMissing(error: unknown): boolean {
  return error instanceof APIError && error.response.status === 404;
}

function isConflict(error: unknown): boolean {
  return error instanceof APIError && error.response.status === 409;
}
