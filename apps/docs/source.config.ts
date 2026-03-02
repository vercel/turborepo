import { remarkMdxMermaid } from "fumadocs-core/mdx-plugins";
import {
  defineConfig,
  defineDocs,
  frontmatterSchema,
  metaSchema,
} from "fumadocs-mdx/config";
import lastModified from "fumadocs-mdx/plugins/last-modified";
import { z } from "zod";

// You can customise Zod schemas for frontmatter and `meta.json` here
// see https://fumadocs.dev/docs/mdx/collections
export const docs = defineDocs({
  dir: "content/docs",
  docs: {
    schema: frontmatterSchema.extend({
      product: z.string().optional(),
      url: z
        .string()
        .regex(/^\/.*/, { message: "url must start with a slash" })
        .optional(),
      type: z
        .enum([
          "conceptual", // Explains what something is and why it exists. Architecture, mental models, design decisions.
          "guide", // Walks through how to accomplish a goal. Tutorials, getting started, workflows.
          "reference", // Lookup-oriented, exhaustive details. API docs, config options, function signatures.
          "troubleshooting", // Diagnoses problems and solutions. FAQs, errors, known issues, debugging guides.
          "integration", // Connects multiple systems. 3rd-party setup, plugins, webhooks, migrations.
          "overview", // High-level introductions. Landing pages, changelogs, release notes.
        ])
        .optional(),
      prerequisites: z
        .array(
          z.string().regex(/^\/.*/, {
            message: "prerequisites must start with a slash",
          })
        )
        .optional(),
      related: z
        .array(
          z
            .string()
            .regex(/^\/.*/, { message: "related must start with a slash" })
        )
        .optional(),
      summary: z.string().optional(),
    }),
    postprocess: {
      includeProcessedMarkdown: true,
    },
  },
  meta: {
    schema: metaSchema,
  },
});

export default defineConfig({
  mdxOptions: {
    remarkPlugins: [remarkMdxMermaid],
  },
  plugins: [lastModified()],
});
