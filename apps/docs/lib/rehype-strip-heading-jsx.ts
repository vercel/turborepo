/**
 * Rehype plugin that strips JSX components from heading IDs and TOC entries.
 *
 * When MDX headings contain inline JSX (e.g., badge components), fumadocs'
 * remarkHeading plugin includes their text content in the generated slug,
 * and rehypeToc includes them in the table of contents.
 *
 * This plugin runs after remarkHeading sets IDs but before rehypeToc reads
 * them. It:
 *  1. Rewrites the heading ID to exclude JSX text
 *  2. Removes JSX elements from heading children (so rehypeToc produces clean titles)
 *  3. Serializes the removed JSX info as a data-heading-badges attribute so
 *     the heading component can re-render them inline
 *
 * Example:
 *   ## Boundaries <ExperimentalBadge>Experimental</ExperimentalBadge>
 *   Before: id="boundaries-experimental", TOC shows badge
 *   After:  id="boundaries", TOC is clean, badge re-injected by component
 */

export interface SerializedBadge {
  component: string;
  text?: string;
}

const HEADING_TAGS = new Set(["h1", "h2", "h3", "h4", "h5", "h6"]);

function flattenNode(node: any): string {
  if ("children" in node && Array.isArray(node.children)) {
    return node.children.map((child: any) => flattenNode(child)).join("");
  }
  if ("value" in node && typeof node.value === "string") {
    return node.value;
  }
  return "";
}

function flattenNodeExcludingJsx(node: any): string {
  if (
    node.type === "mdxJsxTextElement" ||
    node.type === "mdxJsxFlowElement"
  ) {
    return "";
  }
  if ("children" in node && Array.isArray(node.children)) {
    return node.children
      .map((child: any) => flattenNodeExcludingJsx(child))
      .join("");
  }
  if ("value" in node && typeof node.value === "string") {
    return node.value;
  }
  return "";
}

function slugify(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^\p{L}\p{M}\p{N}\p{Pc}\s-]/gu, "")
    .replace(/ /g, "-");
}

function isJsxElement(node: any): boolean {
  return (
    node.type === "mdxJsxTextElement" || node.type === "mdxJsxFlowElement"
  );
}

function hasJsxChild(node: any): boolean {
  return (
    Array.isArray(node.children) &&
    node.children.some((child: any) => isJsxElement(child))
  );
}

function visitHeadings(tree: any, fn: (node: any) => void): void {
  if (!tree || typeof tree !== "object") return;
  if (tree.type === "element" && HEADING_TAGS.has(tree.tagName)) {
    fn(tree);
    return;
  }
  if (Array.isArray(tree.children)) {
    for (const child of tree.children) {
      visitHeadings(child, fn);
    }
  }
}

export default function rehypeStripHeadingJsx() {
  return (tree: any) => {
    const occurrences: Record<string, number> = Object.create(null);

    function uniqueSlug(value: string): string {
      let result = slugify(value);
      const original = result;
      while (Object.prototype.hasOwnProperty.call(occurrences, result)) {
        occurrences[original]++;
        result = `${original}-${occurrences[original]}`;
      }
      occurrences[result] = 0;
      return result;
    }

    visitHeadings(tree, (node) => {
      if (!node.properties?.id) return;

      if (!hasJsxChild(node)) {
        occurrences[node.properties.id] = 0;
        return;
      }

      // Serialize JSX elements so the heading component can re-render them
      const badges: SerializedBadge[] = [];
      for (const child of node.children) {
        if (isJsxElement(child)) {
          const text = flattenNode(child).trim();
          badges.push({
            component: child.name,
            ...(text ? { text } : {}),
          });
        }
      }

      // Remove JSX elements from children so rehypeToc produces clean titles
      node.children = node.children.filter(
        (child: any) => !isJsxElement(child)
      );

      // Store badge data for the heading component to reconstruct
      node.properties["data-heading-badges"] = JSON.stringify(badges);

      // Rewrite the heading ID without JSX text
      const cleanText = flattenNodeExcludingJsx(node).trim();
      node.properties.id = uniqueSlug(cleanText);
    });
  };
}
