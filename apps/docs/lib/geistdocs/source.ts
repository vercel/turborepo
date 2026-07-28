import { createSource } from "@vercel/geistdocs/source";
import {
  type MetaData,
  type PageData,
  type Source,
  loader
} from "fumadocs-core/source";
import { openapiPlugin } from "fumadocs-openapi/server";
import {
  blogDocs,
  blogMeta,
  docs,
  externalBlogDocs,
  externalBlogMeta,
  extraDocs,
  extraMeta,
  openapiDocs,
  openapiMeta
} from "@/.source/server";
import { basePath } from "@/geistdocs";
import { createSignedDocsOgUrl } from "@/lib/og/sign";
import { config } from "./config";

export const geistdocsSource = createSource({
  docs,
  config,
  id: "docs",
  label: "Docs"
});

export const source = geistdocsSource.source;

/**
 * Full LLM-ready markdown for a docs page (frontmatter + processed body +
 * footer links to /sitemap.md, /llms.txt and /agents.md).
 */
export const getLLMText = geistdocsSource.getPageMarkdown;

/**
 * Signed OG image URL for a docs page. The site keeps HMAC-signed OG URLs
 * (see lib/og/sign.ts) instead of the package's unsigned `getPageImage`.
 */
export const getPageImage = (page: { slugs: string[] }) => {
  const segments = [...page.slugs, "image.png"];

  return {
    segments,
    url: createSignedDocsOgUrl(segments, basePath)
  };
};

// Helper function to create source from doc and meta arrays with proper typing
function createLocalSource<
  TPage extends PageData & { info: { path: string; fullPath: string } },
  TMeta extends MetaData & { info: { path: string; fullPath: string } }
>(
  pages: TPage[],
  metas: TMeta[]
): Source<{ pageData: TPage; metaData: TMeta }> {
  const files: Array<
    | { type: "page"; path: string; absolutePath: string; data: TPage }
    | { type: "meta"; path: string; absolutePath: string; data: TMeta }
  > = [];

  for (const entry of pages) {
    files.push({
      type: "page",
      path: entry.info.path,
      absolutePath: entry.info.fullPath,
      data: entry
    });
  }

  for (const entry of metas) {
    files.push({
      type: "meta",
      path: entry.info.path,
      absolutePath: entry.info.fullPath,
      data: entry
    });
  }

  return { files };
}

// Blog loaders
export const blog = loader({
  baseUrl: "/blog",
  source: createLocalSource(blogDocs, blogMeta)
});

export const externalBlog = loader({
  baseUrl: "/blog",
  source: createLocalSource(externalBlogDocs, externalBlogMeta)
});

// OpenAPI loaders
export const openapiPages = loader({
  baseUrl: "/docs/openapi",
  source: createLocalSource(openapiDocs, openapiMeta),
  plugins: [openapiPlugin()]
});

// Extra pages (terms, governance, etc.)
export const extraPages = loader({
  baseUrl: "/",
  source: createLocalSource(extraDocs ?? [], extraMeta ?? [])
});
