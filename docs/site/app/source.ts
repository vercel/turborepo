import { createMDXSource } from "fumadocs-mdx";
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
} from "#.source/index.ts";

export const extraPages = loader({
  baseUrl: "/",
  source: createMDXSource(extrasDocs, extrasMeta),
});

export const repoDocsPages = loader({
  baseUrl: "/docs",
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
