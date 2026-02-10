import { waitUntil } from "@vercel/functions";
import { cronSecret, slackChannel } from "@/lib/env";
import { runSecurityAudit, type AuditResults } from "@/lib/audit";
import { postMessage } from "@/lib/slack";
import type { KnownBlock } from "@slack/web-api";

export const maxDuration = 300;

export async function GET(request: Request) {
  const authHeader = request.headers.get("authorization");
  if (authHeader !== `Bearer ${cronSecret()}`) {
    return new Response("Unauthorized", { status: 401 });
  }

  waitUntil(runAuditAndNotify());
  return Response.json({ ok: true, message: "Audit started" });
}

async function runAuditAndNotify() {
  let results: AuditResults;
  try {
    results = await runSecurityAudit();
  } catch (error) {
    console.error("Audit failed:", error);
    await postMessage(
      slackChannel(),
      ":x: Security audit failed to run. Check the logs."
    );
    return;
  }

  const totalVulns = results.cargo.length + results.pnpm.length;

  if (totalVulns === 0) {
    await postMessage(
      slackChannel(),
      ":white_check_mark: Security audit passed. 0 vulnerabilities found in cargo and pnpm."
    );
    return;
  }

  const blocks: KnownBlock[] = [
    {
      type: "section",
      text: {
        type: "mrkdwn",
        text: `:warning: *Security Audit: ${totalVulns} vulnerabilities found*`
      }
    }
  ];

  if (results.cargo.length > 0) {
    blocks.push(
      { type: "divider" },
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `*Cargo (${results.cargo.length})*\n${results.cargo
            .slice(0, 10)
            .map((v) => `• <${v.url}|${v.name}>: ${v.title}`)
            .join("\n")}`
        }
      }
    );
  }

  if (results.pnpm.length > 0) {
    blocks.push(
      { type: "divider" },
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `*pnpm (${results.pnpm.length})*\n${results.pnpm
            .slice(0, 10)
            .map((v) => `• <${v.url}|${v.name}> (${v.severity}): ${v.title}`)
            .join("\n")}`
        }
      }
    );
  }

  blocks.push(
    { type: "divider" },
    {
      type: "actions",
      elements: [
        {
          type: "button",
          text: { type: "plain_text", text: "Fix vulnerabilities" },
          style: "primary",
          action_id: "audit_fix",
          value: JSON.stringify({
            cargo: results.cargo.length,
            pnpm: results.pnpm.length
          })
        },
        {
          type: "button",
          text: { type: "plain_text", text: "Dismiss" },
          action_id: "audit_dismiss"
        }
      ]
    }
  );

  await postMessage(
    slackChannel(),
    `Security audit: ${totalVulns} vulnerabilities found`,
    blocks
  );
}
