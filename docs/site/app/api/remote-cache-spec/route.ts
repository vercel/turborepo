import { fetchOpenAPISpec } from "@/lib/openapi-spec";

export const revalidate = 0;

export async function GET(): Promise<Response> {
  const spec = await fetchOpenAPISpec();
  return Response.json(spec);
}
