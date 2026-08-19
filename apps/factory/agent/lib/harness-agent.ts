import type { HarnessV1 } from "@ai-sdk/harness";
import { HarnessAgent } from "@ai-sdk/harness/agent";
import { createClaudeCode } from "@ai-sdk/harness-claude-code";
import { createCodex } from "@ai-sdk/harness-codex";
import { createOpenCode } from "@ai-sdk/harness-opencode";
import { createVercelSandbox } from "@ai-sdk/sandbox-vercel";
import { getVercelOidcToken } from "@vercel/oidc";
import { APIError, Sandbox } from "@vercel/sandbox";

import type { HarnessId, SandboxId } from "./harnesses";
import { getGitHubToken } from "./github";

const harnesses: Record<HarnessId, (oidcToken: string) => HarnessV1> = {
  "claude-code": (oidcToken) =>
    createClaudeCode({ auth: { gateway: { apiKey: oidcToken } } }),
  codex: (oidcToken) =>
    createCodex({ auth: { gateway: { apiKey: oidcToken } } }),
  opencode: (oidcToken) =>
    createOpenCode({ auth: { gateway: { apiKey: oidcToken } } })
};

const sandboxes = {
  vercel: (githubToken: string) =>
    createVercelSandbox({
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
      runtime: "node24",
      source: {
        depth: 1,
        revision: "main",
        type: "git",
        url: "https://github.com/vercel/turborepo.git"
      },
      timeout: 45 * 60 * 1000
    })
} satisfies Record<
  SandboxId,
  (githubToken: string) => ReturnType<typeof createVercelSandbox>
>;

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
  const [githubToken, oidcToken] = await Promise.all([
    getGitHubToken(),
    getVercelOidcToken()
  ]);
  return new HarnessAgent({
    harness: harnesses[harnessId](oidcToken),
    id: `turborepo-maintenance-${harnessId}`,
    permissionMode: "allow-all",
    sandbox: sandboxes[sandboxId](githubToken),
    sandboxConfig: {
      bootstrapHash: process.env.VERCEL_GIT_COMMIT_SHA ?? "local",
      workDir: "."
    }
  });
}
