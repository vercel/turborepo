/**
 * DAG layout engine for flowchart diagrams.
 *
 * Adapted from the devtools layout algorithm. Positions nodes by dependency
 * depth — root nodes at the start, downstream nodes further along the
 * primary axis (which depends on graph direction).
 */

import type { Edge, Node } from "reactflow";
import type { FlowDirection, FlowSubgraph } from "./parse-flowchart";

const NODE_HEIGHT = 52;
const MIN_NODE_WIDTH = 120;
const MAX_NODE_WIDTH = 220;
const CHAR_WIDTH = 7.5;
const PADDING = 40;
const VERTICAL_SPACING = 36;
const HORIZONTAL_SPACING = 60;
const SUBGRAPH_PADDING = 24;

function estimateNodeWidth(label: string): number {
  const textWidth = label.length * CHAR_WIDTH + PADDING;
  return Math.min(MAX_NODE_WIDTH, Math.max(MIN_NODE_WIDTH, textWidth));
}

function calculateDepths(
  nodeIds: Set<string>,
  edges: Edge[]
): Map<string, number> {
  const depths = new Map<string, number>();
  const incomingMap = new Map<string, string[]>();

  for (const edge of edges) {
    const existing = incomingMap.get(edge.target);
    if (existing) {
      existing.push(edge.source);
    } else {
      incomingMap.set(edge.target, [edge.source]);
    }
  }

  const roots: string[] = [];
  for (const id of nodeIds) {
    if (!incomingMap.has(id) || incomingMap.get(id)!.length === 0) {
      roots.push(id);
      depths.set(id, 0);
    }
  }

  // BFS to assign depths
  const queue = [...roots];
  while (queue.length > 0) {
    const current = queue.shift()!;
    const currentDepth = depths.get(current) ?? 0;

    for (const edge of edges) {
      if (edge.source === current) {
        const existing = depths.get(edge.target);
        if (existing === undefined || existing < currentDepth + 1) {
          depths.set(edge.target, currentDepth + 1);
          queue.push(edge.target);
        }
      }
    }
  }

  // Orphans get depth 0
  for (const id of nodeIds) {
    if (!depths.has(id)) {
      depths.set(id, 0);
    }
  }

  return depths;
}

export interface LayoutResult {
  nodes: Node[];
  edges: Edge[];
  width: number;
  height: number;
}

export function layoutFlowchart(
  nodes: Node[],
  edges: Edge[],
  direction: FlowDirection,
  subgraphs: FlowSubgraph[] = []
): LayoutResult {
  if (nodes.length === 0) {
    return { nodes: [], edges: [], width: 0, height: 0 };
  }

  const isHorizontal = direction === "LR" || direction === "RL";
  const isReversed = direction === "RL" || direction === "BT";

  const nodeIds = new Set(nodes.map((n) => n.id));
  const depths = calculateDepths(nodeIds, edges);

  // Group nodes by depth
  const byDepth = new Map<number, { node: Node; width: number }[]>();
  for (const node of nodes) {
    const depth = depths.get(node.id) ?? 0;
    const width = estimateNodeWidth(node.data?.label ?? node.id);
    const list = byDepth.get(depth);
    if (list) {
      list.push({ node, width });
    } else {
      byDepth.set(depth, [{ node, width }]);
    }
  }

  const sortedDepths = Array.from(byDepth.keys()).sort((a, b) => a - b);

  // Track subgraph bounding boxes for background rendering
  const subgraphNodeSets = new Map<string, Set<string>>();
  for (const sg of subgraphs) {
    subgraphNodeSets.set(sg.id, new Set(sg.nodeIds));
  }

  const positions = new Map<string, { x: number; y: number; width: number }>();

  if (isHorizontal) {
    // LR/RL: depth advances along X, nodes at same depth stack vertically
    let currentX = 0;

    for (const depth of sortedDepths) {
      const nodesAtDepth = byDepth.get(depth) ?? [];
      let currentY = 0;
      let maxWidth = 0;

      for (const { node, width } of nodesAtDepth) {
        positions.set(node.id, { x: currentX, y: currentY, width });
        currentY += NODE_HEIGHT + VERTICAL_SPACING;
        maxWidth = Math.max(maxWidth, width);
      }

      currentX += maxWidth + HORIZONTAL_SPACING;
    }
  } else {
    // TD/TB/BT: depth advances along Y, nodes at same depth go horizontally
    let currentY = 0;

    for (const depth of sortedDepths) {
      const nodesAtDepth = byDepth.get(depth) ?? [];
      const totalWidth = nodesAtDepth.reduce(
        (sum, n) => sum + n.width + HORIZONTAL_SPACING,
        -HORIZONTAL_SPACING
      );
      let currentX = -totalWidth / 2;

      for (const { node, width } of nodesAtDepth) {
        positions.set(node.id, { x: currentX, y: currentY, width });
        currentX += width + HORIZONTAL_SPACING;
      }

      currentY += NODE_HEIGHT + VERTICAL_SPACING;
    }
  }

  // Apply positions and optionally reverse
  let maxX = 0;
  let maxY = 0;
  let minX = Infinity;

  const layoutedNodes = nodes.map((node) => {
    const pos = positions.get(node.id) ?? { x: 0, y: 0, width: MIN_NODE_WIDTH };
    maxX = Math.max(maxX, pos.x + pos.width);
    maxY = Math.max(maxY, pos.y + NODE_HEIGHT);
    minX = Math.min(minX, pos.x);
    return {
      ...node,
      position: { x: pos.x, y: pos.y },
      style: {
        ...node.style,
        width: pos.width,
        height: NODE_HEIGHT
      }
    };
  });

  // Normalize positions so nothing is negative
  if (minX < 0) {
    for (const node of layoutedNodes) {
      node.position.x -= minX;
    }
    maxX -= minX;
  }

  if (isReversed) {
    if (isHorizontal) {
      for (const node of layoutedNodes) {
        node.position.x =
          maxX -
          node.position.x -
          ((node.style?.width as number) ?? MIN_NODE_WIDTH);
      }
    } else {
      for (const node of layoutedNodes) {
        node.position.y = maxY - node.position.y - NODE_HEIGHT;
      }
    }
  }

  return {
    nodes: layoutedNodes,
    edges,
    width: maxX + SUBGRAPH_PADDING,
    height: maxY + SUBGRAPH_PADDING
  };
}
