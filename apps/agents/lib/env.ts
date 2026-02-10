function required(key: string): string {
  const value = process.env[key];
  if (!value) {
    throw new Error(`Missing required environment variable: ${key}`);
  }
  return value;
}

export function slackBotToken(): string {
  return required("SLACK_BOT_TOKEN");
}

export function slackSigningSecret(): string {
  return required("SLACK_SIGNING_SECRET");
}

export function slackChannel(): string {
  return required("SLACK_CHANNEL");
}

export function githubToken(): string {
  return required("GITHUB_TOKEN");
}

export function githubWebhookSecret(): string {
  return required("GITHUB_WEBHOOK_SECRET");
}

export function cronSecret(): string {
  return required("CRON_SECRET");
}
