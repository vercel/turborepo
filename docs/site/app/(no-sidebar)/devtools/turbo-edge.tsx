import type { CSSProperties } from "react";
import { getBezierPath, useStore, type EdgeProps } from "reactflow";
import { getEdgeParams, type InternalNodeType } from "./floating-edge-utils";

// Helper to resolve marker ID to url(#id) format
function resolveMarker(marker: string | undefined): string | undefined {
  if (typeof marker === "string" && marker && !marker.startsWith("url(")) {
    return `url(#${marker})`;
  }
  return marker;
}

// Offset for arrow marker to prevent line from extending past arrowhead
const ARROW_OFFSET = 10;

export function TurboEdge({
  id,
  source,
  target,
  style = {},
  markerStart,
  markerEnd,
}: EdgeProps) {
  const sourceNode = useStore((store) => store.nodeInternals.get(source));
  const targetNode = useStore((store) => store.nodeInternals.get(target));

  if (!sourceNode || !targetNode) {
    return null;
  }

  const {
    sx: initialSx,
    sy: initialSy,
    tx,
    ty,
    sourcePos,
    targetPos,
  } = getEdgeParams(
    sourceNode as unknown as InternalNodeType,
    targetNode as unknown as InternalNodeType
  );

  let sx = initialSx;
  let sy = initialSy;

  // If there's an arrow marker, shorten the line at the source end
  // so the line doesn't extend past the arrowhead
  const hasArrowMarker =
    typeof markerStart === "string" && markerStart.includes("edge-arrow");
  if (hasArrowMarker) {
    const dx = tx - sx;
    const dy = ty - sy;
    const length = Math.sqrt(dx * dx + dy * dy);
    if (length > 0) {
      const offsetX = (dx / length) * ARROW_OFFSET;
      const offsetY = (dy / length) * ARROW_OFFSET;
      sx += offsetX;
      sy += offsetY;
    }
  }

  const [edgePath]: [string, number, number, number, number] = getBezierPath({
    sourceX: sx,
    sourceY: sy,
    sourcePosition: sourcePos,
    targetPosition: targetPos,
    targetX: tx,
    targetY: ty,
  });

  return (
    <path
      id={id}
      style={style as CSSProperties}
      className="react-flow__edge-path"
      d={edgePath}
      markerStart={resolveMarker(markerStart)}
      markerEnd={resolveMarker(markerEnd)}
    />
  );
}
