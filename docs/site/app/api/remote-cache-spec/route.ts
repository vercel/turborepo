// This file is generated at build time.
import json from "#/.openapi.json";

export const revalidate = 0;

export async function GET(): Promise<Response> {
  return Response.json(json);
}
