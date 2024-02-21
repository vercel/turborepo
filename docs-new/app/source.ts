import { map } from "@/.map";
import { createMDXSource, defaultSchemas } from "fumadocs-mdx";
import { loader } from "fumadocs-core/source";
import { z } from "zod";

const docFrontmatterSchema = defaultSchemas.frontmatter.extend({
  searchable: z.boolean().default(true),
});

export const { getPage, getPages, pageTree } = loader({
  baseUrl: "/repo/docs",
  rootDir: "repo-docs",
  source: createMDXSource(map, {
    schema: { frontmatter: docFrontmatterSchema },
  }),
});

const blogPostFrontmatterSchema = defaultSchemas.frontmatter
  .extend({
    date: z.string(),
    tag: z.string(),
    ogImage: z.string().startsWith("/images/blog/").endsWith("x-card.png"),
  })
  .strict();

export const {
  getPage: getBlogPage,
  getPages: getBlogPages,
  pageTree: blogPageTree,
  files: blogFiles,
} = loader({
  baseUrl: "/blog",
  rootDir: "blog",
  source: createMDXSource(map, {
    // @ts-expect-error -- Doesn't like the usage of strict.
    schema: { frontmatter: blogPostFrontmatterSchema },
  }),
});

const externalBlogPostFrontmatterSchema = defaultSchemas.frontmatter
  .extend({
    date: z.string().optional(),
    isExternal: z.literal(true),
    href: z.string(),
  })
  .strict();

export const {
  getPage: getExternalBlogPage,
  getPages: getExternalBlogPages,
  pageTree: blogExternalPageTree,
  files: blogExternalFiles,
} = loader({
  baseUrl: "/blog",
  rootDir: "external-blog",
  source: createMDXSource(map, {
    // @ts-expect-error -- Doesn't like the usage of strict.
    schema: { frontmatter: externalBlogPostFrontmatterSchema },
  }),
});
