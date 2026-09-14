import { connectLinearCredentials } from "@vercel/connect/eve";
import type {
  LinearChannelCredentials,
  LinearWebhookVerifier
} from "eve/channels/linear";

function requiredEnvironmentVariable(name: string): string {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`Missing required environment variable ${name}.`);
  }
  return value;
}

/**
 * Connect-backed Linear credentials, resolved lazily so a missing
 * connector configuration fails the request that needs it instead of
 * failing at module load. `LINEAR_INSTALLATION_ID` is optional: with a
 * single workspace install Connect resolves it on its own, and setting
 * the variable pins one install when the app is in several workspaces.
 */
function resolveLinearCredentials(): LinearChannelCredentials {
  const installationId = process.env.LINEAR_INSTALLATION_ID?.trim();
  return connectLinearCredentials(
    requiredEnvironmentVariable("LINEAR_CONNECT_UID"),
    installationId ? { installationId } : undefined
  );
}

async function resolveLinearAccessToken(): Promise<string> {
  try {
    const accessToken = resolveLinearCredentials().accessToken;
    if (typeof accessToken === "function") return await accessToken();
    if (accessToken) return accessToken;
  } catch {
    throw new Error("Linear credentials are unavailable.");
  }
  throw new Error("Linear credentials are unavailable.");
}

const verifyLinearWebhook: LinearWebhookVerifier = async (request, body) => {
  try {
    const verifier = resolveLinearCredentials().webhookVerifier;
    return verifier ? await verifier(request, body) : null;
  } catch {
    console.warn("Linear webhook verification failed.");
    return null;
  }
};

export const linearCredentials: LinearChannelCredentials = {
  accessToken: resolveLinearAccessToken,
  webhookVerifier: verifyLinearWebhook
};
