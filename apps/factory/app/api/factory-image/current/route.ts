import { readFactoryImagePointer } from "../../../../agent/lib/factory-image-registry";

/** Read the published image without starting or reconciling image builds. */
export async function GET(): Promise<Response> {
  return Response.json(
    { pointer: await readFactoryImagePointer() },
    { headers: { "cache-control": "no-store" } }
  );
}
