import { map } from "@/.map";
import { createMDXSource, defaultSchemas } from "fumadocs-mdx";
import { loader } from "fumadocs-core/source";
import { ExternalLinkIcon } from "@heroicons/react/outline";
import { z } from "zod";

const docFrontmatterSchema = defaultSchemas.frontmatter.extend({
  searchable: z.boolean().default(true),
});

export const {
  getPage,
  getPages,
  pageTree: repoDocsPageTree,
} = loader({
  baseUrl: "/repo/docs",
  rootDir: "repo-docs",
  source: createMDXSource(map, {
    schema: { frontmatter: docFrontmatterSchema },
  }),
});

// Insert external links into the page tree
repoDocsPageTree.children.splice(9, 0, {
  type: "page",
  name: "Glossary",
  url: "https://vercel.com/docs/vercel-platform/glossary",
  external: true,
  icon: <ExternalLinkIcon />,
});
repoDocsPageTree.children.splice(10, 0, {
  type: "page",
  name: "Changelog",
  url: "https://github.com/vercel/turbo/releases",
  external: true,
  icon: <ExternalLinkIcon />,
});

export const pageTree = repoDocsPageTree;

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
