import {
  type InferPageType,
  type Source,
  type PageData,
  type MetaData,
  loader
} from "fumadocs-core/source";
import { lucideIconsPlugin } from "fumadocs-core/source/lucide-icons";
import { openapiPlugin } from "fumadocs-openapi/server";
import {
  docs,
  blogDocs,
  blogMeta,
  externalBlogDocs,
  externalBlogMeta,
  openapiDocs,
  openapiMeta,
  extraDocs,
  extraMeta
} from "@/.source/server";
import { basePath } from "@/geistdocs";
import { createSignedDocsOgUrl } from "@/lib/og/sign";
import { i18n } from "./i18n";

// Helper function to create source from doc and meta arrays with proper typing
function createSource<
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

// See https://fumadocs.dev/docs/headless/source-api for more info
export const source = loader({
  i18n,
  baseUrl: "/docs",
  source: docs.toFumadocsSource(),
  plugins: [lucideIconsPlugin()]
});

export const getPageImage = (page: InferPageType<typeof source>) => {
  const segments = [...page.slugs, "image.png"];

  return {
    segments,
    url: createSignedDocsOgUrl(segments, basePath)
  };
};

export const getLLMText = async (page: InferPageType<typeof source>) => {
  const processed = await page.data.getText("processed");

  // Clean up the markdown for LLM consumption
  const cleaned = processed
    // Remove import statements
    .replace(/^import\s+.*?from\s+["'].*?["'];?\s*$/gm, "")
    // Collapse multiple consecutive blank lines into a single blank line
    .replace(/\n{3,}/g, "\n\n")
    .trim();

  return `# ${page.data.title}

${cleaned}

---

[View full sitemap](/docs/sitemap.md)`;
};

// Blog loaders
export const blog = loader({
  baseUrl: "/blog",
  source: createSource(blogDocs, blogMeta)
});

export const externalBlog = loader({
  baseUrl: "/blog",
  source: createSource(externalBlogDocs, externalBlogMeta)
});

// OpenAPI loaders
export const openapiPages = loader({
  baseUrl: "/docs/openapi",
  source: createSource(openapiDocs, openapiMeta),
  plugins: [openapiPlugin()]
});

// Extra pages (terms, governance, etc.)
export const extraPages = loader({
  baseUrl: "/",
  source: createSource(extraDocs ?? [], extraMeta ?? [])
});

// Export inferred page types for type-safe usage in components
export type BlogPage = InferPageType<typeof blog>;
export type ExternalBlogPage = InferPageType<typeof externalBlog>;
export type OpenAPIPage = InferPageType<typeof openapiPages>;
export type ExtraPage = InferPageType<typeof extraPages>;
export type DocsPage = InferPageType<typeof source>;
