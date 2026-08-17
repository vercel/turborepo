import { listControlPlaneSnapshot } from "../../../agent/lib/run-registry";

export async function GET(): Promise<Response> {
  return Response.json(await listControlPlaneSnapshot(), {
    headers: { "cache-control": "no-store" }
  });
}
