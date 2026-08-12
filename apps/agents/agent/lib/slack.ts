import { connectSlackCredentials } from "@vercel/connect/eve";
import type {
  SlackBotToken,
  SlackChannelCredentials,
  SlackWebhookVerifier
} from "eve/channels/slack";

function requiredEnvironmentVariable(name: string): string {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`Missing required environment variable ${name}.`);
  }
  return value;
}

function resolveSlackCredentials(): SlackChannelCredentials {
  return connectSlackCredentials(
    requiredEnvironmentVariable("SLACK_CONNECT_UID"),
    {
      installationId: requiredEnvironmentVariable("SLACK_INSTALLATION_ID")
    }
  );
}

async function resolveSlackBotToken(): Promise<string> {
  try {
    const botToken: SlackBotToken | undefined =
      resolveSlackCredentials().botToken;
    if (typeof botToken === "function") return await botToken();
    if (botToken) return botToken;
  } catch {
    throw new Error("Slack credentials are unavailable.");
  }
  throw new Error("Slack credentials are unavailable.");
}

const verifySlackWebhook: SlackWebhookVerifier = async (request, body) => {
  try {
    const verifier = resolveSlackCredentials().webhookVerifier;
    return verifier ? await verifier(request, body) : null;
  } catch {
    console.warn("Slack webhook verification failed.");
    return null;
  }
};

export const slackCredentials: SlackChannelCredentials = {
  botToken: resolveSlackBotToken,
  webhookVerifier: verifySlackWebhook
};

export function slackDestinationChannel(): string {
  return requiredEnvironmentVariable("SLACK_CHANNEL_ID");
}
