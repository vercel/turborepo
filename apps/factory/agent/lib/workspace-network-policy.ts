import type { NetworkPolicy } from "@vercel/sandbox";

import type { WorkspacePublishBridge } from "./workspace-publish.js";

interface WorkspaceNetworkPolicySandbox {
  readonly currentSession: () => {
    readonly update: (input: {
      readonly networkPolicy: NetworkPolicy;
    }) => Promise<void>;
  };
}

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
