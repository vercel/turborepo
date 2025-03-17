import { createMDXSource } from "fumadocs-mdx";
import { createOpenAPI, attachFile } from "fumadocs-openapi/server";
import { loader } from "fumadocs-core/source";
import { openapiDocs, openapiMeta } from "@/.source";

export const openapiPages = loader({
  baseUrl: "/repo/docs/openapi",
  source: createMDXSource(openapiDocs, openapiMeta),
  pageTree: {
    attachFile,
  },
});

export const openapi = createOpenAPI();
