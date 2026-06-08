import { createOpenAPI } from "fumadocs-openapi/server";
import type { OpenAPIV3_2 } from "fumadocs-openapi";
import spec from "./remote-cache-openapi.json";

type Document = OpenAPIV3_2.Document;

const schemaMap = {
  "remote-cache": spec as unknown as Document
} satisfies Record<string, string | Document>;

export const openapi = createOpenAPI({
  input: () => schemaMap
});
