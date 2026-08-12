import { connectSlackCredentials } from "@vercel/connect/eve";

function requiredEnvironmentVariable(name: string): string {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`Missing required environment variable ${name}.`);
  }
  return value;
}

export function slackCredentials() {
  return connectSlackCredentials(
    requiredEnvironmentVariable("SLACK_CONNECT_UID"),
    {
      installationId: requiredEnvironmentVariable("SLACK_INSTALLATION_ID")
    }
  );
}

export function slackDestinationChannel(): string {
  return requiredEnvironmentVariable("SLACK_CHANNEL_ID");
}
