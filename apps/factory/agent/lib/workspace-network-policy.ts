import type { NetworkPolicy } from "@vercel/sandbox";

import type { WorkspacePublishBridge } from "./workspace-publish.js";

interface LegacyNetworkPolicy {
  readonly mode: "custom";
  readonly allowedDomains: readonly string[];
  readonly deniedCIDRs: readonly string[];
  readonly injectionRules: readonly {
    readonly domain: string;
    readonly headers: Readonly<Record<string, string>>;
    readonly match: {
      readonly method: readonly string[];
      readonly path: string | { readonly startsWith: string };
    };
  }[];
}

export function workspaceNetworkPolicy(
  githubToken: string,
  publishBridge?: WorkspacePublishBridge | null
): NetworkPolicy {
  const gitAuthorization = `Basic ${Buffer.from(`x-access-token:${githubToken}`).toString("base64")}`;
  const policy: LegacyNetworkPolicy = {
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
              match: { method: ["POST"], path: publishBridge.path }
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

  // Sandbox API deployments still expect the legacy custom-policy wire shape.
  // The SDK forwards object policies unchanged despite exposing only v2 types.
  return policy as unknown as NetworkPolicy;
}
