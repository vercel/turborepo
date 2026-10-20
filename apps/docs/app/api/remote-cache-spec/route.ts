import spec from "@/lib/remote-cache-openapi.json";

export async function GET(): Promise<Response> {
  return Response.json(spec);
}
