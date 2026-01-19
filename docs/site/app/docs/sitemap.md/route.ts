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
      // Top-level page (e.g., /docs)
      root.push(node);
    } else {
      // Try to find parent
      const parentUrl = "/" + segments.slice(0, -1).join("/");
      const parent = map.get(parentUrl);
      if (parent) {
        parent.children.push(node);
      } else {
        // No direct parent found, add to root
        root.push(node);
      }
    }
  }

  return root;
}

function renderNode(node: PageNode, indent: number): string {
  const prefix = "    ".repeat(indent);
  const lines: string[] = [];

  lines.push(`${prefix}- [${node.title}](${node.url})`);
  lines.push(`${prefix}    - Summary: ${node.description}`);

  for (const child of node.children) {
    lines.push("");
    lines.push(renderNode(child, indent + 1));
  }

  return lines.join("\n");
}

export const GET = async () => {
  const pages = source.getPages();

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
