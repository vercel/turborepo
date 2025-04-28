import { createMDXSource } from "fumadocs-mdx";
import { createOpenAPI } from "fumadocs-openapi/server";
import { loader } from "fumadocs-core/source";
import { openapiDocs, openapiMeta } from "#.source/index.ts";

export const openapiPages = loader({
  baseUrl: "/docs/openapi",
  source: createMDXSource(openapiDocs, openapiMeta),
});

export const openapi = createOpenAPI();
