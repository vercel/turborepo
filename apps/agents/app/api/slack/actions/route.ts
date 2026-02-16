import { verifySlackRequest, updateMessage } from "@/lib/slack";
import { addComment } from "@/lib/github";
import { REPRODUCTION_REQUEST } from "@/lib/templates";

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
  const mention = `<@${payload.user.id}>`;

  switch (action.action_id) {
    case "approve_repro_request": {
      const issueNumber = parseInt(action.value ?? "0", 10);
      if (!issueNumber) break;

      await addComment(issueNumber, REPRODUCTION_REQUEST);
      await updateMessage(
        channel,
        messageTs,
        `:white_check_mark: ${mention} posted reproduction request on <https://github.com/vercel/turborepo/issues/${issueNumber}|#${issueNumber}>`
      );
      break;
    }

    case "repro_dismiss": {
      const issueNumber = parseInt(action.value ?? "0", 10);
      const issueRef = issueNumber
        ? ` for <https://github.com/vercel/turborepo/issues/${issueNumber}|#${issueNumber}>`
        : "";
      await updateMessage(
        channel,
        messageTs,
        `:heavy_minus_sign: ${mention} dismissed${issueRef} — no reproduction request needed`
      );
      break;
    }

    default:
      break;
  }

  return new Response("OK", { status: 200 });
}
