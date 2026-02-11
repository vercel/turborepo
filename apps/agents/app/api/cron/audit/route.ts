import { waitUntil } from "@vercel/functions";
import { cronSecret } from "@/lib/env";
import { runAuditAndFix } from "@/lib/audit";

export const maxDuration = 800;

export async function GET(request: Request) {
  const authHeader = request.headers.get("authorization");
  if (authHeader !== `Bearer ${cronSecret()}`) {
    return new Response("Unauthorized", { status: 401 });
  }

  waitUntil(runAuditAndFix("cron"));
  return Response.json({ ok: true, message: "Audit started" });
}
