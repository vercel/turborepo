import spec from "@/lib/remote-cache-openapi.json";

export const revalidate = 0;

export async function GET(): Promise<Response> {
  return Response.json(spec);
}
