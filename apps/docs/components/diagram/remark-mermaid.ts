/**
 * Remark plugin that transforms ```mermaid code blocks into <Mermaid chart="..." /> JSX elements.
 *
 * Replaces fumadocs-core's remarkMdxMermaid with a version that renders via
 * our custom reactflow-based Mermaid component instead of the mermaid library.
 */

interface CodeNode {
  type: string;
  lang?: string;
  value?: string;
}

interface MdxJsxAttribute {
  type: "mdxJsxAttribute";
  name: string;
  value: string;
}

interface MdxJsxFlowElement {
  type: "mdxJsxFlowElement";
  name: string;
  attributes: MdxJsxAttribute[];
  children: unknown[];
}

interface TreeNode {
  type: string;
  children?: TreeNode[];
}

function remarkMermaid() {
  return (tree: TreeNode) => {
    const children = tree.children;
    if (!children) return;

    for (let i = 0; i < children.length; i++) {
      const node = children[i] as CodeNode;

      if (node.type === "code" && node.lang === "mermaid" && node.value) {
        const jsxNode: MdxJsxFlowElement = {
          type: "mdxJsxFlowElement",
          name: "Mermaid",
          attributes: [
            {
              type: "mdxJsxAttribute",
              name: "chart",
              value: node.value
            }
          ],
          children: []
        };

        (children as unknown[])[i] = jsxNode;
      }
    }
  };
}

export default remarkMermaid;
