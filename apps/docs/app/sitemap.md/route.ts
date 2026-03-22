import { source } from "@/lib/geistdocs/source";

export const revalidate = false;

interface PageNode {
  title: string;
  description: string;
  url: string;
  children: PageNode[];
}

function buildTree(
  pages: Array<{ url: string; data: { title: string; description?: string } }>
): PageNode[] {
  const root: PageNode[] = [];
  const map = new Map<string, PageNode>();

  // Sort pages by URL to ensure parents come before children
  const sorted = [...pages].sort((a, b) => a.url.localeCompare(b.url));

  for (const page of sorted) {
    const node: PageNode = {
      title: page.data.title,
      description: page.data.description ?? "",
      url: page.url,
      children: []
    };
    map.set(page.url, node);

    // Find parent by removing last segment
    const segments = page.url.split("/").filter(Boolean);
    if (segments.length <= 1) {
      root.push(node);
    } else {
      const parentUrl = "/" + segments.slice(0, -1).join("/");
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

function inferDocType(url: string): string {
  if (url.includes("/getting-started")) return "Tutorial";
  if (url.includes("/reference/")) return "Reference";
  if (url.includes("/guides/")) return "How-to";
  return "Conceptual";
}

function extractTopics(url: string): string[] {
  const segments = url
    .replace(/^\/docs\/?/, "")
    .split("/")
    .filter(Boolean);
  return segments.slice(0, 3);
}

function truncateToWords(text: string, maxWords: number): string {
  const words = text.split(/\s+/);
  if (words.length <= maxWords) return text;
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
  segments.push(`Type: ${inferDocType(node.url)}`);
  if (node.description) {
    segments.push(`Summary: ${truncateToWords(node.description, 100)}`);
  }
  if (parentTitle) {
    segments.push(`Prerequisites: ${parentTitle}`);
  }
  const topics = extractTopics(node.url);
  if (topics.length > 0) {
    segments.push(`Topics: ${topics.join(", ")}`);
  }

  lines.push(`${prefix}- [${node.title}](${node.url}) | ${segments.join(" | ")}`);

  for (const child of node.children) {
    lines.push("");
    lines.push(renderNode(child, indent + 1, node.title));
  }

  return lines.join("\n");
}

export const GET = async () => {
  const pages = source.getPages("en");

  const tree = buildTree(pages);

  const header = `# Turborepo Documentation Sitemap

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
      "Content-Type": "text/markdown"
    }
  });
};
