import { waitUntil } from "@vercel/functions";
import { cronSecret, slackChannel } from "@/lib/env";
import { runSecurityAudit, runAuditFix, type AuditResults } from "@/lib/audit";
import { postMessage, updateMessage } from "@/lib/slack";
import type { KnownBlock } from "@slack/web-api";

export const maxDuration = 800;

// Vercel Cron uses GET with Authorization header
export async function GET(request: Request) {
  const authHeader = request.headers.get("authorization");
  if (authHeader !== `Bearer ${cronSecret()}`) {
    return new Response("Unauthorized", { status: 401 });
  }

  waitUntil(runAuditAndFix());
  return Response.json({ ok: true, message: "Audit started" });
}

// UI trigger uses POST with CRON_SECRET in the body
export async function POST(request: Request) {
  const body = await request.json();
  if (body.secret !== cronSecret()) {
    return new Response("Unauthorized", { status: 401 });
  }

  waitUntil(runAuditAndFix());
  return Response.json({ ok: true, message: "Audit started" });
}

async function runAuditAndFix() {
  // 1. Run the audit scan
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

  // 2. Post initial status, then immediately start the fix agent
  const statusMsg = await postMessage(
    slackChannel(),
    `:hourglass_flowing_sand: Security audit found ${totalVulns} vulnerabilities. Fix agent is running...`
  );

  const statusChannel = slackChannel();
  const statusTs = statusMsg.ts as string;

  const onProgress = async (message: string) => {
    await updateMessage(
      statusChannel,
      statusTs,
      `:hourglass_flowing_sand: ${message}`
    );
  };

  // 3. Run the fix agent
  try {
    const fixResult = await runAuditFix(onProgress);
    const { agentResults: r, branch, diff } = fixResult;

    const statusLine = [
      `${r.vulnerabilitiesFixed} fixed, ${r.vulnerabilitiesRemaining} remaining`,
      `tests: ${r.testsPass ? "passing" : "failing"}`,
      `audits: ${r.auditsClean ? "clean" : "not clean"}`
    ].join(" · ");

    const diffPreview =
      diff.length > 2500 ? diff.slice(0, 2500) + "\n... (truncated)" : diff;

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

    // Replace the status message with the results
    await updateMessage(
      statusChannel,
      statusTs,
      `Audit fix ready for review (branch: ${branch})`,
      blocks
    );
  } catch (error) {
    console.error("Audit fix agent failed:", error);
    const msg = error instanceof Error ? error.message : String(error);
    await updateMessage(
      statusChannel,
      statusTs,
      `:x: Audit fix agent failed: ${msg}`
    );
  }
}
