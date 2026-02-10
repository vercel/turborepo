import { waitUntil } from "@vercel/functions";
import { verifySlackRequest, updateMessage, postMessage } from "@/lib/slack";
import { slackChannel } from "@/lib/env";
import { addComment } from "@/lib/github";
import { runAuditFix, openFixPR, type AgentResults } from "@/lib/audit";
import { REPRODUCTION_REQUEST } from "@/lib/templates";
import type { KnownBlock } from "@slack/web-api";

interface SlackAction {
  type: string;
  action_id: string;
  value?: string;
}

interface SlackActionPayload {
  type: "block_actions";
  user: { id: string; username: string };
  actions: SlackAction[];
  channel: { id: string };
  message: { ts: string; text: string };
}

export const maxDuration = 300;

export async function POST(request: Request) {
  const rawBody = await request.text();
  await verifySlackRequest(request, rawBody);

  const params = new URLSearchParams(rawBody);
  const payload = JSON.parse(
    params.get("payload") ?? "{}"
  ) as SlackActionPayload;

  if (payload.type !== "block_actions") {
    return new Response("OK", { status: 200 });
  }

  const action = payload.actions[0];
  if (!action) return new Response("OK", { status: 200 });

  const channel = payload.channel.id;
  const messageTs = payload.message.ts;
  const user = payload.user.username;

  switch (action.action_id) {
    // Issue triage: user approves posting a reproduction request comment
    case "approve_repro_request": {
      const issueNumber = parseInt(action.value ?? "0", 10);
      if (!issueNumber) break;

      await addComment(issueNumber, REPRODUCTION_REQUEST);
      await updateMessage(
        channel,
        messageTs,
        `:white_check_mark: @${user} posted reproduction request on #${issueNumber}`
      );
      break;
    }

    case "repro_dismiss": {
      await updateMessage(
        channel,
        messageTs,
        `:heavy_minus_sign: @${user} dismissed — no reproduction request needed`
      );
      break;
    }

    // Audit flow step 1: user triggers the fix agent
    case "audit_fix": {
      await updateMessage(
        channel,
        messageTs,
        `:hourglass_flowing_sand: @${user} triggered the audit fix agent...`
      );

      const onProgress = async (message: string) => {
        await updateMessage(
          channel,
          messageTs,
          `:hourglass_flowing_sand: @${user} — ${message}`
        );
      };

      waitUntil(
        runAuditFix(onProgress)
          .then(async (result) => {
            const { agentResults: r, branch, diff } = result;
            const statusLine = [
              `${r.vulnerabilitiesFixed} fixed, ${r.vulnerabilitiesRemaining} remaining`,
              `tests: ${r.testsPass ? "passing" : "failing"}`,
              `audits: ${r.auditsClean ? "clean" : "not clean"}`
            ].join(" · ");

            // Truncate diff for Slack (max ~3000 chars in a code block)
            const diffPreview =
              diff.length > 2500
                ? diff.slice(0, 2500) + "\n... (truncated)"
                : diff;

            const blocks: KnownBlock[] = [
              {
                type: "section",
                text: {
                  type: "mrkdwn",
                  text: `:white_check_mark: *Audit fix agent finished*\n${statusLine}`
                }
              },
              {
                type: "section",
                text: {
                  type: "mrkdwn",
                  text: `*Summary:* ${r.summary}`
                }
              },
              {
                type: "section",
                text: {
                  type: "mrkdwn",
                  text: `\`\`\`\n${diffPreview}\n\`\`\``
                }
              },
              {
                type: "actions",
                elements: [
                  {
                    type: "button",
                    text: { type: "plain_text", text: "Open PR" },
                    style: "primary",
                    action_id: "audit_open_pr",
                    value: JSON.stringify({ branch, agentResults: r })
                  },
                  {
                    type: "button",
                    text: { type: "plain_text", text: "Dismiss" },
                    action_id: "audit_dismiss"
                  }
                ]
              }
            ];

            await postMessage(
              slackChannel(),
              `Audit fix ready for review (branch: ${branch})`,
              blocks
            );

            await updateMessage(
              channel,
              messageTs,
              `:white_check_mark: @${user} — agent finished. Review posted above.`
            );
          })
          .catch(async (error: Error) => {
            console.error("Audit fix agent failed:", error);
            await updateMessage(
              channel,
              messageTs,
              `:x: @${user} audit fix agent failed: ${error.message}`
            );
          })
      );
      break;
    }

    // Audit flow step 2: user approves opening the PR
    case "audit_open_pr": {
      let parsed: { branch: string; agentResults: AgentResults };
      try {
        parsed = JSON.parse(action.value ?? "{}");
      } catch {
        await updateMessage(
          channel,
          messageTs,
          `:x: Failed to parse action data`
        );
        break;
      }

      try {
        const pr = await openFixPR(parsed.branch, parsed.agentResults);
        await updateMessage(
          channel,
          messageTs,
          `:white_check_mark: @${user} opened <${pr.prUrl}|PR #${pr.prNumber}>`
        );
      } catch (error) {
        const msg = error instanceof Error ? error.message : String(error);
        await updateMessage(
          channel,
          messageTs,
          `:x: @${user} failed to open PR: ${msg}`
        );
      }
      break;
    }

    case "audit_dismiss": {
      await updateMessage(
        channel,
        messageTs,
        `:heavy_minus_sign: @${user} dismissed the audit results`
      );
      break;
    }

    default:
      break;
  }

  return new Response("OK", { status: 200 });
}
