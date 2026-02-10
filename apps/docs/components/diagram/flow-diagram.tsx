"use client";

import { useTheme } from "next-themes";
import { useEffect, useMemo, useState } from "react";
import ReactFlow, {
  Controls,
  type Edge,
  MarkerType,
  type Node,
  Position,
  Handle
} from "reactflow";
import "reactflow/dist/style.css";
import type { FlowchartGraph, FlowNode as ParsedNode } from "./parse-flowchart";
import { layoutFlowchart } from "./layout";

function shapeToStyle(shape: ParsedNode["shape"]): React.CSSProperties {
  switch (shape) {
    case "circle":
      return { borderRadius: "50%" };
    case "diamond":
      return { borderRadius: 4, transform: "rotate(45deg)" };
    case "stadium":
      return { borderRadius: 999 };
    case "round":
      return { borderRadius: 12 };
    case "subroutine":
      return { borderRadius: 4, borderWidth: 3 };
    default:
      return { borderRadius: 6 };
  }
}

interface DiagramNodeData {
  label: string;
  shape: ParsedNode["shape"];
}

function DiagramNode({ data }: { data: DiagramNodeData }) {
  const isDiamond = data.shape === "diamond";

  return (
    <div
      className="diagram-node"
      style={{
        ...shapeToStyle(data.shape),
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: "8px 16px",
        height: "100%",
        width: "100%",
        backgroundColor: "var(--diagram-node-bg)",
        border: "1.5px solid var(--diagram-node-border)",
        color: "var(--diagram-text)",
        fontSize: 13,
        fontFamily: "var(--font-mono, monospace)",
        lineHeight: 1.3,
        textAlign: "center"
      }}
    >
      <Handle
        type="target"
        id="top"
        position={Position.Top}
        style={{ opacity: 0 }}
      />
      <Handle
        type="target"
        id="left"
        position={Position.Left}
        style={{ opacity: 0 }}
      />
      <Handle
        type="target"
        id="bottom"
        position={Position.Bottom}
        style={{ opacity: 0 }}
      />
      <Handle
        type="target"
        id="right"
        position={Position.Right}
        style={{ opacity: 0 }}
      />
      <span style={isDiamond ? { transform: "rotate(-45deg)" } : undefined}>
        {data.label}
      </span>
      <Handle
        type="source"
        id="top"
        position={Position.Top}
        style={{ opacity: 0 }}
      />
      <Handle
        type="source"
        id="left"
        position={Position.Left}
        style={{ opacity: 0 }}
      />
      <Handle
        type="source"
        id="bottom"
        position={Position.Bottom}
        style={{ opacity: 0 }}
      />
      <Handle
        type="source"
        id="right"
        position={Position.Right}
        style={{ opacity: 0 }}
      />
    </div>
  );
}

const nodeTypes = { diagram: DiagramNode };

function getHandleIds(direction: FlowchartGraph["direction"]): {
  sourceHandle: string;
  targetHandle: string;
} {
  switch (direction) {
    case "LR":
      return { sourceHandle: "right", targetHandle: "left" };
    case "RL":
      return { sourceHandle: "left", targetHandle: "right" };
    case "BT":
      return { sourceHandle: "top", targetHandle: "bottom" };
    case "TD":
    case "TB":
    default:
      return { sourceHandle: "bottom", targetHandle: "top" };
  }
}

function graphToReactFlow(graph: FlowchartGraph): {
  nodes: Node[];
  edges: Edge[];
} {
  const { sourceHandle, targetHandle } = getHandleIds(graph.direction);

  const nodes: Node[] = graph.nodes.map((n) => ({
    id: n.id,
    type: "diagram",
    data: { label: n.label, shape: n.shape } satisfies DiagramNodeData,
    position: { x: 0, y: 0 }
  }));

  const edges: Edge[] = graph.edges.map((e, i) => ({
    id: `e-${e.source}-${e.target}-${i}`,
    source: e.source,
    target: e.target,
    sourceHandle,
    targetHandle,
    label: e.label,
    type: "default",
    animated: e.style === "dotted",
    style: {
      strokeWidth: e.style === "thick" ? 3 : 1.5,
      strokeDasharray: e.style === "dotted" ? "5 3" : undefined,
      stroke: "var(--diagram-edge)"
    },
    labelStyle: {
      fontSize: 11,
      fill: "var(--diagram-text)"
    },
    markerEnd:
      e.style !== "plain"
        ? { type: MarkerType.ArrowClosed, color: "var(--diagram-edge)" }
        : undefined
  }));

  return { nodes, edges };
}

interface FlowDiagramProps {
  graph: FlowchartGraph;
}

export function FlowDiagram({ graph }: FlowDiagramProps) {
  const [mounted, setMounted] = useState(false);
  const { resolvedTheme } = useTheme();

  useEffect(() => {
    setMounted(true);
  }, []);

  const { nodes, edges, width, height } = useMemo(() => {
    const { nodes: rawNodes, edges: rawEdges } = graphToReactFlow(graph);
    return layoutFlowchart(
      rawNodes,
      rawEdges,
      graph.direction,
      graph.subgraphs
    );
  }, [graph]);

  if (!mounted) return null;

  const isDark = resolvedTheme === "dark";

  return (
    <div
      className="my-6 rounded-lg border overflow-hidden"
      style={{
        height: Math.max(200, height + 80),
        ["--diagram-node-bg" as string]: isDark
          ? "rgb(30, 30, 30)"
          : "rgb(250, 250, 250)",
        ["--diagram-node-border" as string]: isDark
          ? "rgb(60, 60, 60)"
          : "rgb(200, 200, 200)",
        ["--diagram-text" as string]: isDark
          ? "rgb(220, 220, 220)"
          : "rgb(30, 30, 30)",
        ["--diagram-edge" as string]: isDark
          ? "rgb(120, 120, 120)"
          : "rgb(150, 150, 150)",
        backgroundColor: isDark ? "rgb(17, 17, 17)" : "rgb(255, 255, 255)"
      }}
    >
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        fitView
        fitViewOptions={{ padding: 0.3 }}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        zoomOnScroll={false}
        panOnDrag={false}
        proOptions={{ hideAttribution: true }}
        minZoom={0.3}
        maxZoom={1.5}
      >
        <Controls
          showInteractiveToggle={false}
          style={{
            borderColor: isDark ? "rgb(60, 60, 60)" : "rgb(200, 200, 200)"
          }}
        />
      </ReactFlow>
    </div>
  );
}
