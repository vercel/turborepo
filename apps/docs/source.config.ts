import {
  transformerNotationDiff,
  transformerNotationFocus,
  transformerNotationHighlight,
  transformerNotationWordHighlight
} from "@shikijs/transformers";
import {
  defineGeistdocsSourceConfig,
  geistShikiTheme,
  geistdocsFrontmatterSchema,
  geistdocsMetaSchema
} from "@vercel/geistdocs/source-config";
import { defineDocs } from "fumadocs-mdx/config";
import type { ShikiTransformer } from "shiki";
import { z } from "zod";
import rehypeStripHeadingJsx from "./lib/rehype-strip-heading-jsx";

const transformerAddLanguage: ShikiTransformer = {
  name: "add-language-attribute",
  pre(node) {
    if (this.options.lang) {
      node.properties["data-language"] = this.options.lang;
    }
  }
};

// You can customise Zod schemas for frontmatter and `meta.json` here
// see https://fumadocs.dev/docs/mdx/collections
export const docs = defineDocs({
  dir: "content/docs",
  docs: {
    schema: geistdocsFrontmatterSchema,
    postprocess: {
      includeProcessedMarkdown: true
    }
  },
  meta: {
    schema: geistdocsMetaSchema
  }
});

export const { docs: blogDocs, meta: blogMeta } = defineDocs({
  dir: "content/blog",
  docs: {
    schema: geistdocsFrontmatterSchema
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
    schema: geistdocsFrontmatterSchema.extend({
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
    schema: geistdocsFrontmatterSchema.extend({
      description: z.string()
    })
  }
});

export default defineGeistdocsSourceConfig({
  mdxOptions: {
    rehypePlugins: [rehypeStripHeadingJsx],
    rehypeCodeOptions: {
      // defineGeistdocsSourceConfig sets these at runtime; repeated here to
      // satisfy the fumadocs RehypeCodeOptions type, which requires themes.
      themes: {
        light: geistShikiTheme,
        dark: geistShikiTheme
      },
      transformers: [
        transformerNotationHighlight({ matchAlgorithm: "v3" }),
        transformerNotationWordHighlight({ matchAlgorithm: "v3" }),
        transformerNotationDiff({ matchAlgorithm: "v3" }),
        transformerNotationFocus({ matchAlgorithm: "v3" }),
        transformerAddLanguage
      ]
    }
  }
});
