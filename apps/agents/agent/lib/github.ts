import { getVercelOidcToken } from "@vercel/oidc";
import type { GitHubChannelCredentials } from "eve/channels/github";

const TOKEN_EXPIRY_SAFETY_WINDOW_MS = 30_000;

interface GitHubStsResponse {
  token: string;
  expires_at: string;
}

let cachedToken: { token: string; expiresAt: number } | null = null;

export const githubCredentials: GitHubChannelCredentials = {
  installationToken: getGitHubToken
};

export async function getGitHubToken(): Promise<string> {
  if (
    cachedToken !== null &&
    cachedToken.expiresAt - TOKEN_EXPIRY_SAFETY_WINDOW_MS > Date.now()
  ) {
    return cachedToken.token;
  }

  const oidcToken = await getVercelOidcToken();
  const tokenExchangeUrl = process.env.GITHUB_TOKEN_EXCHANGE_URL;
  if (!tokenExchangeUrl) {
    throw new Error("GITHUB_TOKEN_EXCHANGE_URL is not configured.");
  }
  const response = await fetch(tokenExchangeUrl, {
    method: "POST",
    headers: {
      authorization: `Bearer ${oidcToken}`,
      "content-type": "application/json"
    },
    body: JSON.stringify({
      owner: "vercel",
      repo: "turborepo",
      permissions: {
        contents: "write",
        pull_requests: "write"
      }
    })
  });

  if (!response.ok) {
    const detail = await response.text().catch(() => "");
    throw new Error(
      `GitHub token exchange failed (${response.status} ${response.statusText})${detail === "" ? "" : `: ${detail}`}`
    );
  }

  const body = (await response.json()) as Partial<GitHubStsResponse>;
  if (typeof body.token !== "string" || body.token === "") {
    throw new Error("GitHub token exchange did not include a token.");
  }
  const expiresAt = Date.parse(String(body.expires_at));
  if (Number.isNaN(expiresAt)) {
    throw new TypeError(
      `GitHub token exchange had invalid expires_at: ${String(body.expires_at)}`
    );
  }

  cachedToken = { token: body.token, expiresAt };
  return body.token;
}
