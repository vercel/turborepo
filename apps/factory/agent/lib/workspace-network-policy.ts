import type { NetworkPolicy } from "@vercel/sandbox";

import type { WorkspacePublishBridge } from "./workspace-publish.js";

export function workspaceNetworkPolicy(
  githubToken: string,
  publishBridge?: WorkspacePublishBridge | null
): NetworkPolicy {
  const gitAuthorization = `Basic ${Buffer.from(`x-access-token:${githubToken}`).toString("base64")}`;
  return {
    allow: {
      ...(publishBridge
        ? {
            [publishBridge.hostname]: [
              {
                match: { method: ["POST"], path: publishBridge.path },
                transform: [
                  { headers: { authorization: publishBridge.authorization } }
                ]
              }
            ]
          }
        : {}),
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
          transform: [{ headers: { authorization: gitAuthorization } }]
        }
      ],
      "*": []
    },
    subnets: { deny: ["169.254.169.254/32"] }
  };
}
