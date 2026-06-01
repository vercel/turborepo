import { createOpenAPI, type OpenAPIOptions } from "fumadocs-openapi/server";
import spec from "./remote-cache-openapi.json";

type SchemaMap = Exclude<NonNullable<OpenAPIOptions["input"]>, string[]> extends
  () => infer T
  ? Awaited<T>
  : never;

const schemaMap: SchemaMap = {
  // The checked-in spec is OpenAPI 3.0.3, while the library types its in-memory
  // documents as 3.2.x. Runtime ingestion still supports the older schema.
  "remote-cache": spec as unknown as SchemaMap[string]
};

export const openapi = createOpenAPI({
  input: () => schemaMap
});
