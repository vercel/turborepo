import { fetchGatewayModels } from "../../../agent/lib/gateway-models";

/** Tool-capable language models currently available through AI Gateway. */
export async function GET(): Promise<Response> {
  try {
    return Response.json(
      { models: await fetchGatewayModels() },
      { headers: { "cache-control": "public, max-age=300" } }
    );
  } catch (error) {
    console.error("Could not fetch AI Gateway models.", error);
    return Response.json(
      { error: "Could not fetch the available models." },
      { status: 503 }
    );
  }
}
