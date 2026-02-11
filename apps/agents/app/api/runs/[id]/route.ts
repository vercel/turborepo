import { getRun } from "@/lib/runs";

export const dynamic = "force-dynamic";

export async function GET(
  _request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const run = await getRun(id);
  if (!run) {
    return Response.json({ error: "Run not found" }, { status: 404 });
  }
  return Response.json(run);
}
