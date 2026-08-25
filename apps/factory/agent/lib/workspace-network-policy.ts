import type { NetworkPolicy } from "@vercel/sandbox";

import type { WorkspacePublishBridge } from "./workspace-publish.js";

interface WorkspaceNetworkPolicySandbox {
  readonly currentSession: () => {
    readonly update: (input: {
      readonly networkPolicy: NetworkPolicy;
    }) => Promise<void>;
  };
}

interface WorkspaceGitAuthenticationSandbox {
  readonly runCommand: (input: {
    readonly args: string[];
    readonly cmd: string;
    readonly cwd: string;
    readonly timeoutMs: number;
  }) => Promise<{
    readonly exitCode: number;
    readonly stderr: () => Promise<string>;
  }>;
}

const GITHUB_GIT_AUTHORIZATION = `Basic ${Buffer.from(
  "x-access-token:brokered-by-sandbox"
).toString("base64")}`;

interface SandboxApiNetworkPolicy {
  readonly mode: "custom";
  readonly allowedDomains: readonly string[];
  readonly deniedCIDRs: readonly string[];
  readonly injectionRules: readonly {
    readonly domain: string;
    readonly headers: Readonly<Record<string, string>>;
    readonly match: {
      readonly method: readonly string[];
      readonly path:
        | { readonly exact: string }
        | { readonly startsWith: string };
    };
  }[];
}

export async function applyWorkspaceNetworkPolicy(
  sandbox: WorkspaceNetworkPolicySandbox,
  networkPolicy: SandboxApiNetworkPolicy
): Promise<void> {
  await sandbox.currentSession().update({
    networkPolicy: networkPolicy as unknown as NetworkPolicy
  });
}

/**
 * Makes Git send an Authorization header on its first GitHub request. The
 * value is deliberately unusable: Sandbox network policy replaces it with
 * the brokered installation credential without exposing that credential to
 * the workspace. Without a seed header, Git attempts an interactive username
 * prompt after GitHub's 401 response and non-interactive pushes fail.
 */
export async function configureWorkspaceGitAuthentication(
  sandbox: WorkspaceGitAuthenticationSandbox,
  checkout = "/factory/turborepo"
): Promise<void> {
  const command = await sandbox.runCommand({
    args: [
      "config",
      "--local",
      "http.https://github.com/.extraheader",
      `Authorization: ${GITHUB_GIT_AUTHORIZATION}`
    ],
    cmd: "git",
    cwd: checkout,
    timeoutMs: 10_000
  });
  if (command.exitCode !== 0) {
    throw new Error(
      `Could not configure workspace GitHub authentication: ${await command.stderr()}`
    );
  }
}

export function workspaceNetworkPolicy(
  githubToken: string,
  publishBridge?: WorkspacePublishBridge | null
): SandboxApiNetworkPolicy {
  const gitAuthorization = `Basic ${Buffer.from(`x-access-token:${githubToken}`).toString("base64")}`;
  return {
    mode: "custom",
    allowedDomains: [
      ...(publishBridge ? [publishBridge.hostname] : []),
      "api.github.com",
      "github.com",
      "*"
    ],
    deniedCIDRs: ["169.254.169.254/32"],
    injectionRules: [
      ...(publishBridge
        ? [
            {
              domain: publishBridge.hostname,
              headers: { authorization: publishBridge.authorization },
              match: {
                method: ["POST"],
                path: { exact: publishBridge.path }
              }
            }
          ]
        : []),
      {
        domain: "api.github.com",
        headers: { authorization: `Bearer ${githubToken}` },
        match: {
          method: ["GET", "POST", "PATCH"],
          path: { startsWith: "/repos/vercel/turborepo" }
        }
      },
      {
        domain: "github.com",
        headers: { authorization: gitAuthorization },
        match: {
          method: ["GET", "POST"],
          path: { startsWith: "/vercel/turborepo.git" }
        }
      }
    ]
  };
}
