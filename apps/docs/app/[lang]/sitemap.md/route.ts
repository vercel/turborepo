import type { NextRequest } from "next/server";
import { source } from "@/lib/geistdocs/source";

export const revalidate = false;

const DOCS_PREFIX_PATTERN = /^\/docs\/?/;
const WHITESPACE_PATTERN = /\s+/;

type PageNode = {
  title: string;
  description: string;
  url: string;
  type?: string;
  summary?: string;
  prerequisites?: string[];
  product?: string;
  lastmod?: string;
  children: PageNode[];
};

function buildTree(
  pages: Array<{
    url: string;
    data: {
      title: string;
      description?: string;
      type?: string;
      summary?: string;
      prerequisites?: string[];
      product?: string;
      lastModified?: Date;
    };
  }>
): PageNode[] {
  const root: PageNode[] = [];
  const map = new Map<string, PageNode>();

  const sorted = [...pages].sort((a, b) => a.url.localeCompare(b.url));

  for (const page of sorted) {
    const node: PageNode = {
      title: page.data.title,
      description: page.data.description ?? "",
      url: page.url,
      type: page.data.type,
      summary: page.data.summary,
      prerequisites: page.data.prerequisites,
      product: page.data.product,
      lastmod: page.data.lastModified
        ? page.data.lastModified.toISOString().split("T")[0]
        : undefined,
      children: [],
    };
    map.set(page.url, node);

    const segments = page.url.split("/").filter(Boolean);
    if (segments.length <= 1) {
      root.push(node);
    } else {
      const parentUrl = `/${segments.slice(0, -1).join("/")}`;
      const parent = map.get(parentUrl);
      if (parent) {
        parent.children.push(node);
      } else {
        root.push(node);
      }
    }
  }

  return root;
}

function inferDocType(url: string, explicitType?: string): string {
  if (explicitType) {
    return explicitType.charAt(0).toUpperCase() + explicitType.slice(1);
  }
  if (url.includes("/getting-started")) {
    return "Guide";
  }
  if (url.includes("/reference")) {
    return "Reference";
  }
  if (url.includes("/guides/")) {
    return "Guide";
  }
  return "Conceptual";
}

function extractTopics(url: string, product?: string): string[] {
  const topics: string[] = [];
  if (product) {
    topics.push(product);
  }

  const segments = url
    .replace(DOCS_PREFIX_PATTERN, "")
    .split("/")
    .filter(Boolean);

  for (const segment of segments) {
    if (!topics.includes(segment)) {
      topics.push(segment);
    }
    if (topics.length >= 3) {
      break;
    }
  }

  return topics.slice(0, 3);
}

function truncateToWords(text: string, maxWords: number): string {
  const words = text.split(WHITESPACE_PATTERN);
  if (words.length <= maxWords) {
    return text;
  }
  return `${words.slice(0, maxWords).join(" ")}...`;
}

function renderNode(
  node: PageNode,
  indent: number,
  parentTitle?: string
): string {
  const prefix = "    ".repeat(indent);
  const lines: string[] = [];

  const segments: string[] = [];
  segments.push(`Type: ${inferDocType(node.url, node.type)}`);

  if (node.lastmod) {
    segments.push(`Lastmod: ${node.lastmod}`);
  }

  const summary = node.summary || node.description;
  if (summary) {
    segments.push(`Summary: ${truncateToWords(summary, 100)}`);
  }

  const prereqs =
    node.prerequisites && node.prerequisites.length > 0
      ? node.prerequisites.join(", ")
      : parentTitle;
  if (prereqs) {
    segments.push(`Prerequisites: ${prereqs}`);
  }

  const topics = extractTopics(node.url, node.product);
  if (topics.length > 0) {
    segments.push(`Topics: ${topics.join(", ")}`);
  }

  lines.push(
    `${prefix}- [${node.title}](${node.url}) | ${segments.join(" | ")}`
  );

  for (const child of node.children) {
    lines.push("");
    lines.push(renderNode(child, indent + 1, node.title));
  }

  return lines.join("\n");
}

export const GET = async (
  _req: NextRequest,
  { params }: RouteContext<"/[lang]/sitemap.md">
) => {
  const { lang } = await params;
  const pages = source.getPages(lang);

  const tree = buildTree(pages);

  const header = `# Documentation Sitemap

## Purpose

This file is a high-level semantic index of the documentation.
It is intended for:

- LLM-assisted navigation (ChatGPT, Claude, etc.)
- Quick orientation for contributors
- Identifying relevant documentation areas during development

It is not intended to replace individual docs.

---

`;

  const body = tree.map((node) => renderNode(node, 0)).join("\n\n");

  return new Response(header + body, {
    headers: {
      "Content-Type": "text/markdown",
    },
  });
};
