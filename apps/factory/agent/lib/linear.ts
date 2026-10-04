import { connect, connectLinearCredentials } from "@vercel/connect/eve";
import {
  type LinearChannelCredentials,
  type LinearWebhookVerifier
} from "eve/channels/linear";

function requiredEnvironmentVariable(name: string): string {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`Missing required environment variable ${name}.`);
  }
  return value;
}

// Resolve lazily so builds and unrelated Factory channels do not require
// Linear configuration. Connect keeps tokens and webhook verification off
// the model-facing surface.
export function createLinearCredentials(
  resolveCredentials: () => LinearChannelCredentials = () =>
    connectLinearCredentials(requiredEnvironmentVariable("LINEAR_CONNECT_UID"))
): LinearChannelCredentials {
  const verifyWebhook: LinearWebhookVerifier = async (request, body) => {
    try {
      const verifier = resolveCredentials().webhookVerifier;
      return verifier ? await verifier(request, body) : null;
    } catch {
      console.warn("Linear webhook verification failed.");
      return null;
    }
  };

  return {
    async accessToken() {
      try {
        const token = resolveCredentials().accessToken;
        const value = typeof token === "function" ? await token() : token;
        if (value) return value;
      } catch {
        // Do not expose provider errors or credentials in Agent Activities.
      }
      throw new Error("Linear credentials are unavailable.");
    },
    webhookVerifier: verifyWebhook
  };
}

export const linearCredentials = createLinearCredentials();

export function linearMcpAuth() {
  // The MCP service uses a separate Connect client from the Linear agent app.
  // App scope also supports scheduled/internal turns with no user principal.
  return connect({
    connector: requiredEnvironmentVariable("LINEAR_MCP_CONNECT_UID"),
    principalType: "app"
  });
}
