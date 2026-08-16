import { HarnessAgent } from "@ai-sdk/harness/agent";
import { createJustBashSandbox } from "@ai-sdk/sandbox-just-bash";

import { createRemoteOpenCode } from "./remote-opencode-harness";

const TURBOREPO_LOCATION = "/workspace/projects/turborepo";

function serverHeaders(): HeadersInit {
  const headers = new Headers();
  const token = process.env.OPENCODE_SERVER_TOKEN;
  const password = process.env.OPENCODE_SERVER_PASSWORD;
  if ((token ? 1 : 0) + (password ? 1 : 0) !== 1) {
    throw new Error(
      "Configure exactly one of OPENCODE_SERVER_TOKEN or OPENCODE_SERVER_PASSWORD"
    );
  }
  if (token) headers.set("authorization", `Bearer ${token}`);
  if (password) {
    headers.set(
      "authorization",
      `Basic ${Buffer.from(`opencode:${password}`).toString("base64")}`
    );
  }
  return headers;
}

export function createRemoteOpenCodeAgent(title: string) {
  const baseURL = process.env.OPENCODE_SERVER_URL;
  if (!baseURL) throw new Error("OPENCODE_SERVER_URL is required");

  return new HarnessAgent({
    harness: createRemoteOpenCode({
      baseURL,
      headers: serverHeaders,
      location: { directory: TURBOREPO_LOCATION },
      title
    }),
    sandbox: createJustBashSandbox()
  });
}
