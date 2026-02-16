import { listRuns } from "@/lib/runs";

export const dynamic = "force-dynamic";

export async function GET() {
  const runs = await listRuns();
  return Response.json(runs);
}
