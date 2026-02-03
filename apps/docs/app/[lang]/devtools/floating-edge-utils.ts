import { Position } from "reactflow";

// Type for node structure from reactflow v11 nodeInternals
export interface InternalNodeType {
  id: string;
  width?: number;
  height?: number;
  positionAbsolute?: {
    x: number;
    y: number;
  };
  position: {
    x: number;
    y: number;
  };
}

// Get the center point of a node
function getNodeCenter(node: InternalNodeType) {
  const width = node.width ?? 0;
  const height = node.height ?? 0;
  const pos = node.positionAbsolute ?? node.position;
  const x = pos.x + width / 2;
  const y = pos.y + height / 2;

  return { x, y };
}

// Get the intersection point of the line from center to target with the node border
function getNodeIntersection(
  intersectionNode: InternalNodeType,
  targetNode: InternalNodeType
) {
  const width = intersectionNode.width ?? 1;
  const height = intersectionNode.height ?? 1;
  const nodeCenter = getNodeCenter(intersectionNode);
  const targetCenter = getNodeCenter(targetNode);

  const w = width / 2;
  const h = height / 2;

  const dx = targetCenter.x - nodeCenter.x;
  const dy = targetCenter.y - nodeCenter.y;

  // Prevent division by zero
  if (dx === 0 && dy === 0) {
    return { x: nodeCenter.x, y: nodeCenter.y };
  }

  // Handle perfectly vertical lines (dx === 0)
  if (dx === 0) {
    return {
      x: nodeCenter.x,
      y: dy > 0 ? nodeCenter.y + h : nodeCenter.y - h
    };
  }

  const slope = Math.abs(dy / dx);
  const nodeSlope = h / w;

  let x: number;
  let y: number;

  if (slope <= nodeSlope) {
    // Intersects left or right edge
    x = dx > 0 ? nodeCenter.x + w : nodeCenter.x - w;
    y = nodeCenter.y + (dy * w) / Math.abs(dx);
  } else {
    // Intersects top or bottom edge
    x = nodeCenter.x + (dx * h) / Math.abs(dy);
    y = dy > 0 ? nodeCenter.y + h : nodeCenter.y - h;
  }

  return { x, y };
}

// Get the position (TOP, RIGHT, BOTTOM, LEFT) based on intersection point
function getEdgePosition(
  node: InternalNodeType,
  intersectionPoint: { x: number; y: number }
): Position {
  const width = node.width ?? 0;
  const height = node.height ?? 0;
  const pos = node.positionAbsolute ?? node.position;
  const nx = pos.x;
  const ny = pos.y;

  const px = Math.round(intersectionPoint.x);
  const py = Math.round(intersectionPoint.y);

  if (px <= Math.round(nx + 1)) {
    return Position.Left;
  }
  if (px >= Math.round(nx + width - 1)) {
    return Position.Right;
  }
  if (py <= Math.round(ny + 1)) {
    return Position.Top;
  }
  if (py >= Math.round(ny + height - 1)) {
    return Position.Bottom;
  }

  return Position.Top;
}

// Get all params needed to draw an edge between two nodes
export function getEdgeParams(
  source: InternalNodeType,
  target: InternalNodeType
) {
  const sourceIntersection = getNodeIntersection(source, target);
  const targetIntersection = getNodeIntersection(target, source);

  const sourcePos = getEdgePosition(source, sourceIntersection);
  const targetPos = getEdgePosition(target, targetIntersection);

  return {
    sx: sourceIntersection.x,
    sy: sourceIntersection.y,
    tx: targetIntersection.x,
    ty: targetIntersection.y,
    sourcePos,
    targetPos
  };
}
