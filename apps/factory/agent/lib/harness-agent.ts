import type { HarnessV1 } from "@ai-sdk/harness";
import { HarnessAgent } from "@ai-sdk/harness/agent";
import { createClaudeCode } from "@ai-sdk/harness-claude-code";
import { createCodex } from "@ai-sdk/harness-codex";
import { createOpenCode } from "@ai-sdk/harness-opencode";
import { createVercelSandbox } from "@ai-sdk/sandbox-vercel";
import { getVercelOidcToken } from "@vercel/oidc";
import { APIError, Sandbox } from "@vercel/sandbox";
import type { Experimental_SandboxSession } from "ai";

import { FACTORY_IMAGE_SPEC, factoryImageFingerprint } from "./factory-image";
import { readFactoryImagePointer } from "./factory-image-registry";
import type { HarnessId, SandboxId } from "./harnesses";
import { getGitHubToken } from "./github";

/** Published factory image a Harness session boots from. */
interface HarnessSandboxImage {
  readonly commit: string;
  readonly snapshotId: string;
}

const SANDBOX_TIMEOUT_MS = 45 * 60 * 1000;
const REPOSITORY_URL = "https://github.com/vercel/turborepo.git";

const harnesses: Record<HarnessId, (oidcToken: string) => HarnessV1> = {
  "claude-code": (oidcToken) =>
    createClaudeCode({ auth: { gateway: { apiKey: oidcToken } } }),
  codex: (oidcToken) =>
    createCodex({ auth: { gateway: { apiKey: oidcToken } } }),
  opencode: (oidcToken) =>
    createOpenCode({ auth: { gateway: { apiKey: oidcToken } } })
};

const sandboxes = {
  vercel: (githubToken: string, image: HarnessSandboxImage | null) => {
    const shared = {
      env: { GH_TOKEN: "brokered-by-sandbox" },
      networkPolicy: {
        allow: {
          "*": [],
          "api.github.com": [
            {
              match: {
                method: ["GET", "POST", "PATCH"],
                path: { startsWith: "/repos/vercel/turborepo" }
              },
              transform: [
                { headers: { authorization: `Bearer ${githubToken}` } }
              ]
            }
          ],
          "github.com": [
            {
              match: {
                method: ["GET", "POST"],
                path: { startsWith: "/vercel/turborepo.git" }
              },
              transform: [
                { headers: { authorization: `Bearer ${githubToken}` } }
              ]
            }
          ]
        },
        subnets: { deny: ["169.254.169.254/32"] }
      },
      ports: [4000],
      timeout: SANDBOX_TIMEOUT_MS
    };
    // A snapshot source is mutually exclusive with a stock runtime and a
    // git source: the factory image already carries the checkout and the
    // toolchain, so only the fallback needs to clone.
    return image === null
      ? createVercelSandbox({
          ...shared,
          runtime: "node24",
          source: {
            depth: 1,
            revision: "main",
            type: "git",
            url: REPOSITORY_URL
          }
        })
      : createVercelSandbox({
          ...shared,
          source: { snapshotId: image.snapshotId, type: "snapshot" }
        });
  }
} satisfies Record<
  SandboxId,
  (
    githubToken: string,
    image: HarnessSandboxImage | null
  ) => ReturnType<typeof createVercelSandbox>
>;

/**
 * Factory image to boot from, or `null` when none matches this
 * deployment's toolchain and the session should clone instead.
 */
async function resolveHarnessImage(): Promise<HarnessSandboxImage | null> {
  const pointer = await readFactoryImagePointer();
  if (pointer === null || pointer.fingerprint !== factoryImageFingerprint()) {
    return null;
  }
  return { commit: pointer.commit, snapshotId: pointer.snapshotId };
}

export async function createHarnessAgent(
  harnessId: HarnessId,
  sandboxId: SandboxId,
  sessionId?: string
) {
  if (sessionId) {
    try {
      await (
        await Sandbox.get({ name: `ai-sdk-harness-session-${sessionId}` })
      ).delete();
    } catch (error) {
      if (!(error instanceof APIError && error.response.status === 404))
        throw error;
    }
  }
  const [githubToken, oidcToken, image] = await Promise.all([
    getGitHubToken(),
    getVercelOidcToken(),
    resolveHarnessImage()
  ]);
  return new HarnessAgent({
    harness: harnesses[harnessId](oidcToken),
    id: `turborepo-maintenance-${harnessId}`,
    permissionMode: "allow-all",
    sandbox: sandboxes[sandboxId](githubToken, image),
    sandboxConfig: {
      // Rotate the Harness template whenever the factory image changes.
      bootstrapHash:
        image?.snapshotId ?? process.env.VERCEL_GIT_COMMIT_SHA ?? "local",
      onSession: image === null ? undefined : fastForwardCheckout,
      // The factory image links its canonical checkout into the sandbox
      // working directory; the clone fallback lands there directly.
      workDir: image === null ? "." : checkoutDirectoryName()
    }
  });
}

function checkoutDirectoryName(): string {
  return FACTORY_IMAGE_SPEC.checkoutPath.split("/").at(-1) as string;
}

/**
 * Brings the image's checkout up to the current `main` before the harness
 * starts. A stale checkout is far better than a failed run, so problems
 * are logged rather than thrown.
 */
async function fastForwardCheckout({
  session
}: {
  readonly session: Experimental_SandboxSession;
}): Promise<void> {
  const repository = FACTORY_IMAGE_SPEC.checkoutPath;
  try {
    await session.run({
      command: `git -C ${repository} fetch --depth=1 --force origin main && git -C ${repository} reset --hard FETCH_HEAD && git -C ${repository} clean -ffd && cd ${repository} && pnpm install --frozen-lockfile`
    });
  } catch (error) {
    console.error("Could not fast-forward the factory checkout.", error);
  }
}
