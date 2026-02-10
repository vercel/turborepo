import { waitUntil } from "@vercel/functions";
import { verifyGitHubWebhook } from "@/lib/github";
import { postMessage } from "@/lib/slack";
import { slackChannel } from "@/lib/env";
import { classifyIssueForReproduction } from "@/lib/ai";
import type { KnownBlock } from "@slack/web-api";

interface GitHubIssuePayload {
  action: string;
  issue: {
    number: number;
    title: string;
    html_url: string;
    body: string | null;
    user: { login: string };
    labels: Array<{ name: string }>;
    pull_request?: unknown;
  };
  sender: { login: string };
}

export async function POST(request: Request) {
  const rawBody = await request.text();

  try {
    await verifyGitHubWebhook(request, rawBody);
  } catch {
    return new Response("Unauthorized", { status: 401 });
  }

  const event = request.headers.get("x-github-event");
  const payload = JSON.parse(rawBody);

  if (event === "issues" && payload.action === "opened") {
    waitUntil(triageNewIssue(payload as GitHubIssuePayload));
  }

  return new Response("OK", { status: 200 });
}

async function triageNewIssue(payload: GitHubIssuePayload) {
  const { issue } = payload;

  if (issue.pull_request) return;

  let classification;
  try {
    classification = await classifyIssueForReproduction(
      issue.title,
      issue.body ?? ""
    );
  } catch (error) {
    console.error("Failed to classify issue:", error);
    return;
  }

  if (!classification.needsReproduction) return;

  const bodyPreview = issue.body
    ? issue.body.slice(0, 300) + (issue.body.length > 300 ? "..." : "")
    : "_No description provided_";

  const blocks: KnownBlock[] = [
    {
      type: "section",
      text: {
        type: "mrkdwn",
        text: `:mag: *Missing reproduction: <${issue.html_url}|#${issue.number} ${issue.title}>*\nby ${issue.user.login}`
      }
    },
    {
      type: "context",
      elements: [
        {
          type: "mrkdwn",
          text: `_Agent reasoning: ${classification.reasoning}_`
        }
      ]
    },
    {
      type: "actions",
      elements: [
        {
          type: "button",
          text: { type: "plain_text", text: "Request reproduction" },
          style: "primary",
          action_id: "approve_repro_request",
          value: String(issue.number)
        },
        {
          type: "button",
          text: { type: "plain_text", text: "Dismiss" },
          action_id: "repro_dismiss",
          value: String(issue.number)
        }
      ]
    }
  ];

  await postMessage(
    slackChannel(),
    `Missing reproduction on #${issue.number}: ${issue.title}`,
    blocks
  );
}
