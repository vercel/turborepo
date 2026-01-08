import { remarkMdxMermaid } from "fumadocs-core/mdx-plugins";
import {
  defineConfig,
  defineDocs,
  frontmatterSchema,
  metaSchema
} from "fumadocs-mdx/config";
import lastModified from "fumadocs-mdx/plugins/last-modified";
import { createCssVariablesTheme } from "shiki";
import { z } from "zod";

// You can customise Zod schemas for frontmatter and `meta.json` here
// see https://fumadocs.dev/docs/mdx/collections
export const docs = defineDocs({
  dir: "content/docs",
  docs: {
    schema: frontmatterSchema,
    postprocess: {
      includeProcessedMarkdown: true
    }
  },
  meta: {
    schema: metaSchema
  }
});

export const { docs: blogDocs, meta: blogMeta } = defineDocs({
  dir: "content/blog",
  docs: {
    schema: frontmatterSchema
      .extend({
        description: z.string(),
        date: z.string(),
        tag: z.string(),
        ogImage: z
          .string()
          .startsWith("/images/blog/")
          .endsWith("x-card.png")
          .optional()
      })
      .strict()
  }
});

export const { docs: externalBlogDocs, meta: externalBlogMeta } = defineDocs({
  dir: "content/external-blog",
  docs: {
    schema: frontmatterSchema.extend({
      description: z.string(),
      date: z.string(),
      isExternal: z.literal(true),
      href: z.string()
    })
  }
});

export const { docs: openapiDocs, meta: openapiMeta } = defineDocs({
  dir: "content/openapi"
});

export const { docs: extraDocs, meta: extraMeta } = defineDocs({
  dir: "content/extra",
  docs: {
    schema: frontmatterSchema.extend({
      description: z.string()
    })
  }
});

const theme = createCssVariablesTheme({
  name: "css-variables",
  variablePrefix: "--shiki-",
  variableDefaults: {}
});

export default defineConfig({
  mdxOptions: {
    remarkPlugins: [remarkMdxMermaid],
    rehypeCodeOptions: {
      themes: {
        light: theme,
        dark: theme
      }
    }
  },
  plugins: [lastModified()]
});
