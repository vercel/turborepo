import { handleTerminalRequest } from "@/agent/lib/sandbox-terminal";

export const dynamic = "force-dynamic";

export async function POST(request: Request): Promise<Response> {
  if (
    request.headers.get("origin") !== new URL(request.url).origin ||
    request.headers.get("content-type")?.split(";", 1)[0] !== "application/json"
  )
    return Response.json(
      { error: "Invalid terminal request." },
      { status: 403 }
    );

  return handleTerminalRequest(request);
}
