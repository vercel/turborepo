import { WebClient, type KnownBlock } from "@slack/web-api";
import crypto from "node:crypto";
import { slackBotToken, slackSigningSecret } from "./env";

let _client: WebClient | undefined;
export function slackClient(): WebClient {
  if (!_client) {
    _client = new WebClient(slackBotToken());
  }
  return _client;
}

export async function verifySlackRequest(
  request: Request,
  rawBody: string
): Promise<void> {
  const timestamp = request.headers.get("x-slack-request-timestamp");
  const signature = request.headers.get("x-slack-signature");

  if (!timestamp || !signature) {
    throw new Error("Missing Slack verification headers");
  }

  const fiveMinutesAgo = Math.floor(Date.now() / 1000) - 60 * 5;
  if (parseInt(timestamp, 10) < fiveMinutesAgo) {
    throw new Error("Slack request too old");
  }

  const sigBasestring = `v0:${timestamp}:${rawBody}`;
  const hmac = crypto.createHmac("sha256", slackSigningSecret());
  hmac.update(sigBasestring);
  const computedSignature = `v0=${hmac.digest("hex")}`;

  if (
    !crypto.timingSafeEqual(
      Buffer.from(signature),
      Buffer.from(computedSignature)
    )
  ) {
    throw new Error("Invalid Slack signature");
  }
}

export async function postMessage(
  channel: string,
  text: string,
  blocks?: KnownBlock[]
) {
  return slackClient().chat.postMessage({
    channel,
    text,
    blocks,
    unfurl_links: false
  });
}

export async function updateMessage(
  channel: string,
  ts: string,
  text: string,
  blocks?: KnownBlock[]
) {
  return slackClient().chat.update({
    channel,
    ts,
    text,
    blocks
  });
}
