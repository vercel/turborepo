import type { NetworkPolicy } from "@vercel/sandbox";

export function workspaceNetworkPolicy(githubToken: string): NetworkPolicy {
  const gitAuthorization = `Basic ${Buffer.from(`x-access-token:${githubToken}`).toString("base64")}`;
  return {
    allow: {
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
