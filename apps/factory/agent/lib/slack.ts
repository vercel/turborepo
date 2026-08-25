import { connectSlackCredentials } from "@vercel/connect/eve";
import {
  callSlackApi,
  type SlackBotToken,
  type SlackChannelCredentials,
  type SlackWebhookVerifier
} from "eve/channels/slack";

const defaultTimeoutMs = 5000;

interface SlackApiResponse {
  readonly channel?: unknown;
  readonly error?: unknown;
  readonly ok?: boolean;
  readonly ts?: unknown;
}

interface SlackMessageInput {
  readonly channel: string;
  readonly text: string;
  readonly threadTimestamp?: string;
}

export interface SlackDeliveryOptions {
  readonly channel?: string;
  readonly event: string;
  readonly logger?: Pick<Console, "error" | "info">;
  readonly metadata?: Readonly<Record<string, string | number | null>>;
  readonly send?: (input: SlackMessageInput) => Promise<SlackApiResponse>;
  readonly timeoutMs?: number;
}

export type SlackDeliveryResult =
  | {
      readonly ok: true;
      readonly channel: string;
      readonly timestamp: string | null;
    }
  | {
      readonly ok: false;
      readonly channel: string | null;
      readonly error: string;
    };

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

export function slackDestinationChannel(): string {
  const channel = process.env.SLACK_CHANNEL_ID?.trim();
  if (!channel) throw new Error("Slack destination is unavailable.");
  return channel;
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

async function sendSlackMessage({
  channel,
  text,
  threadTimestamp
}: SlackMessageInput) {
  return callSlackApi({
    botToken: slackCredentials.botToken,
    operation: "chat.postMessage",
    body: {
      channel,
      text,
      ...(threadTimestamp ? { thread_ts: threadTimestamp } : {})
    }
  });
}

function errorMessage(error: unknown): string {
  if (
    error instanceof Error &&
    (error.message ===
      "Slack delivery status is unknown because the request timed out." ||
      error.message === "Slack credentials are unavailable." ||
      error.message === "Slack destination is unavailable." ||
      error.message === "Slack API rejected the message." ||
      /^Slack API returned [a-z0-9_]+\.$/.test(error.message))
  ) {
    return error.message;
  }
  return "Slack delivery failed before the API accepted the message.";
}

function logDelivery(
  logger: Pick<Console, "error" | "info">,
  level: "error" | "info",
  message: string,
  details: Record<string, unknown>
) {
  try {
    logger[level](message, details);
  } catch {
    // Delivery diagnostics must not change the delivery result.
  }
}

export async function deliverSlackMessage(
  text: string,
  options: SlackDeliveryOptions & { readonly threadTimestamp?: string }
): Promise<SlackDeliveryResult> {
  const logger = options.logger ?? console;
  let channel: string | null = null;
  let timeout: NodeJS.Timeout | undefined;

  try {
    channel = options.channel ?? slackDestinationChannel();
    const response = await Promise.race([
      (options.send ?? sendSlackMessage)({
        channel,
        text,
        threadTimestamp: options.threadTimestamp
      }),
      new Promise<never>((_, reject) => {
        timeout = setTimeout(
          () =>
            reject(
              new Error(
                "Slack delivery status is unknown because the request timed out."
              )
            ),
          options.timeoutMs ?? defaultTimeoutMs
        );
      })
    ]);

    if (response.ok !== true) {
      throw new Error(
        typeof response.error === "string"
          ? `Slack API returned ${response.error}.`
          : "Slack API rejected the message."
      );
    }

    const result: SlackDeliveryResult = {
      ok: true,
      channel:
        typeof response.channel === "string" ? response.channel : channel,
      timestamp: typeof response.ts === "string" ? response.ts : null
    };
    logDelivery(logger, "info", "Slack message delivered.", {
      event: options.event,
      channel: result.channel,
      timestamp: result.timestamp,
      ...options.metadata
    });
    return result;
  } catch (error) {
    const result: SlackDeliveryResult = {
      ok: false,
      channel,
      error: errorMessage(error)
    };
    logDelivery(logger, "error", "Slack message delivery failed.", {
      event: options.event,
      channel: result.channel,
      error: result.error,
      ...options.metadata
    });
    return result;
  } finally {
    if (timeout) clearTimeout(timeout);
  }
}

interface UnsafeIssueAlert {
  readonly issueNumber: number;
  readonly issueTitle: string;
  readonly issueUrl: string;
  readonly reason: string;
}

export type UnsafeIssueAlertResult =
  | { readonly ok: true; readonly channel: string; readonly timestamp: string }
  | { readonly ok: false; readonly error: string };

export async function alertUnsafeIssue(
  issue: UnsafeIssueAlert,
  options: Omit<SlackDeliveryOptions, "event" | "metadata"> = {}
): Promise<UnsafeIssueAlertResult> {
  const metadata = { issueNumber: issue.issueNumber, issueUrl: issue.issueUrl };
  const root = await deliverSlackMessage(
    `:warning: Factory blocked <${issue.issueUrl}|#${issue.issueNumber}: ${issue.issueTitle}> during security triage.`,
    { ...options, event: "unsafe_issue_alert", metadata }
  );
  if (!root.ok) return { ok: false, error: root.error };
  if (root.timestamp === null) {
    return { ok: false, error: "Slack did not return a thread timestamp." };
  }

  const reply = await deliverSlackMessage(
    `Why it was blocked: ${issue.reason}`,
    {
      ...options,
      channel: root.channel,
      event: "unsafe_issue_alert_reason",
      metadata,
      threadTimestamp: root.timestamp
    }
  );
  if (!reply.ok) return { ok: false, error: reply.error };
  return { ok: true, channel: root.channel, timestamp: root.timestamp };
}
