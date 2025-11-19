import {
  defineDocs,
  defineConfig,
  frontmatterSchema,
} from "fumadocs-mdx/config";
import { z } from "zod";
import { createCssVariablesTheme } from "shiki";

export const { docs: repoDocs, meta: repoMeta } = defineDocs({
  dir: "content/docs",
  docs: {
    schema: frontmatterSchema,
  },
});

export const { docs: extrasDocs, meta: extrasMeta } = defineDocs({
  dir: "content/extra",
  docs: {
    schema: frontmatterSchema.extend({
      description: z.string(),
    }),
  },
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
          .optional(),
      })
      .strict(),
  },
});

export const { docs: externalBlogDocs, meta: externalBlogMeta } = defineDocs({
  dir: "content/external-blog",
  docs: {
    schema: frontmatterSchema.extend({
      description: z.string(),
      date: z.string(),
      isExternal: z.literal(true),
      href: z.string(),
    }),
  },
});

export const { docs: openapiDocs, meta: openapiMeta } = defineDocs({
  dir: "content/openapi",
});

const theme = createCssVariablesTheme({
  name: "css-variables",
  variablePrefix: "--shiki-",
  variableDefaults: {},
});

export default defineConfig({
  mdxOptions: {
    rehypeCodeOptions: {
      themes: {
        light: theme,
        dark: theme,
      },
    },
  },
});
