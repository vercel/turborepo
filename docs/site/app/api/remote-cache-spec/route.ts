import json from "#.openapi.json";

export const revalidate = 0;

export function GET(): Response {
  return Response.json(json);
}
