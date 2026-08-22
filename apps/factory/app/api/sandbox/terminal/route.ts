import { handleTerminalRequest } from "@/agent/lib/sandbox-terminal";

export const dynamic = "force-dynamic";

export async function POST(request: Request): Promise<Response> {
  return handleTerminalRequest(request);
}
