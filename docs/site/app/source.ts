import { createMDXSource } from "fumadocs-mdx";
// import { createOpenAPI, attachFile } from "fumadocs-openapi/server";
import { loader } from "fumadocs-core/source";
import {
  repoDocs,
  repoMeta,
  extrasDocs,
  extrasMeta,
  blogDocs,
  blogMeta,
  externalBlogDocs,
  externalBlogMeta,
  openapiDocs,
  openapiMeta,
} from "@/.source";

export const extraPages = loader({
  baseUrl: "/",
  source: createMDXSource(extrasDocs, extrasMeta),
});

export const repoDocsPages = loader({
  baseUrl: "/repo/docs",
  source: createMDXSource(repoDocs, repoMeta),
});

export const blog = loader({
  baseUrl: "/blog",
  source: createMDXSource(blogDocs, blogMeta),
});

export const externalBlog = loader({
  baseUrl: "/blog",
  source: createMDXSource(externalBlogDocs, externalBlogMeta),
});

// export const openapiPages = loader({
//   baseUrl: "/repo/docs/openapi",
//   source: createMDXSource(openapiDocs, openapiMeta),
//   pageTree: {
//     attachFile,
//   },
// });

// export const openapi = createOpenAPI();
