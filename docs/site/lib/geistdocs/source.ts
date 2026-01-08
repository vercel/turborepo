import { type InferPageType, loader } from "fumadocs-core/source";
import { lucideIconsPlugin } from "fumadocs-core/source/lucide-icons";
import {
  docs,
  blogDocs,
  blogMeta,
  externalBlogDocs,
  externalBlogMeta
} from "@/.source/server";
import { basePath } from "@/geistdocs";
import { i18n } from "./i18n";

// Helper function to create source from doc and meta arrays
function createSource(pages: typeof blogDocs, metas: typeof blogMeta) {
  const files: Array<{
    type: "page" | "meta";
    path: string;
    absolutePath: string;
    data: (typeof pages)[number] | (typeof metas)[number];
  }> = [];

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
    url: basePath
      ? `${basePath}/og/${segments.join("/")}`
      : `/og/${segments.join("/")}`
  };
};

export const getLLMText = async (page: InferPageType<typeof source>) => {
  const processed = await page.data.getText("processed");

  return `# ${page.data.title}

${processed}`;
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
