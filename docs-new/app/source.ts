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

const blogFrontmatterSchema = defaultSchemas.frontmatter.extend({
  date: z.date(),
  tag: z.string(),
  ogImage: z.string().startsWith("/images/blog/turbo").endsWith("x-card.png"),
  href: z.string().optional(),
});

export const {
  getPage: getBlogPage,
  getPages: getBlogPages,
  pageTree: blogPageTree,
  files: blogFiles,
} = loader({
  baseUrl: "/blog",
  rootDir: "blog",
  source: createMDXSource(map, {
    schema: { frontmatter: blogFrontmatterSchema },
  }),
});
