import { getBezierPath, useStore, type EdgeProps } from "reactflow";
import { getEdgeParams, type InternalNodeType } from "./floating-edge-utils";

export default function TurboEdge({
  id,
  source,
  target,
  style = {},
  markerEnd,
}: EdgeProps) {
  const sourceNode = useStore((store) => store.nodeInternals.get(source));
  const targetNode = useStore((store) => store.nodeInternals.get(target));

  if (!sourceNode || !targetNode) {
    return null;
  }

  const { sx, sy, tx, ty, sourcePos, targetPos } = getEdgeParams(
    sourceNode as unknown as InternalNodeType,
    targetNode as unknown as InternalNodeType
  );

  const [edgePath] = getBezierPath({
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
      style={style}
      className="react-flow__edge-path"
      d={edgePath}
      markerEnd={markerEnd}
    />
  );
}
