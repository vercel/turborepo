import { getLogs } from "@/lib/runs";

export const dynamic = "force-dynamic";

export async function GET(
  _request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const logs = await getLogs(id);
  return new Response(logs, {
    headers: { "Content-Type": "text/plain" }
  });
}
