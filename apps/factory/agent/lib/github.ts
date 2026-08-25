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

/**
 * Current `main` head. Used when a factory image build is requested
 * without a webhook payload to name the revision. Authenticates with an
 * installation token when one is configured, and otherwise reads the
 * public repository anonymously.
 */
export async function fetchMainCommit(): Promise<string> {
  const headers: Record<string, string> = {
    accept: "application/vnd.github+json",
    "x-github-api-version": "2022-11-28"
  };
  const token = await getGitHubToken().catch(() => null);
  if (token !== null) headers.authorization = `Bearer ${token}`;

  const response = await fetch(
    "https://api.github.com/repos/vercel/turborepo/commits/main",
    { headers }
  );
  if (!response.ok) {
    throw new Error(
      `Could not resolve vercel/turborepo main (${response.status} ${response.statusText}).`
    );
  }
  const body = (await response.json()) as { sha?: unknown };
  if (typeof body.sha !== "string" || !/^[0-9a-f]{40}$/.test(body.sha)) {
    throw new TypeError("GitHub returned no commit SHA for main.");
  }
  return body.sha;
}

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
        issues: "write",
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
