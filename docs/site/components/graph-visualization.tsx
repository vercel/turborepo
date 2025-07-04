"use client";

import { useState, useRef, useMemo, useEffect } from "react";
import {
  ReactFlow,
  Controls,
  Background,
  Handle,
  Position,
} from "@xyflow/react";
import type { Node } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { Card } from "./card";
import { Button } from "./button";
import { Callout } from "./callout";

interface GraphData {
  nodes: Array<{
    id: string;
    label: string;
    name?: string;
  }>;
  edges: Array<{
    source: string;
    target: string;
  }>;
}

interface GraphVisualizationProps {
  className?: string;
  initialData?: string | null;
}

// Custom node component for better styling
const CustomNode = ({
  data,
}: {
  data: {
    label: string;
    isRoot?: boolean;
    isHighlighted?: boolean;
    isFaded?: boolean;
  };
}) => {
  const getNodeClasses = () => {
    const baseClasses =
      "relative px-3 py-2 rounded-lg border-2 font-medium text-sm shadow-sm transition-all duration-200";
    const colorClasses = data.isRoot
      ? "bg-red-500 text-white border-red-600"
      : "bg-blue-500 text-white border-blue-600";

    let stateClasses = "opacity-100";
    if (data.isFaded) {
      stateClasses = "opacity-10";
    } else if (data.isHighlighted) {
      stateClasses = "opacity-100 ring-2 ring-yellow-400 ring-offset-2";
    }

    return `${baseClasses} ${colorClasses} ${stateClasses}`;
  };

  return (
    <div className={getNodeClasses()} style={{ minWidth: 80, minHeight: 40 }}>
      {/* Handles on all four sides */}
      <Handle type="target" position={Position.Top} id="top" />
      <Handle type="target" position={Position.Left} id="left" />
      <Handle type="target" position={Position.Right} id="right" />
      <Handle type="target" position={Position.Bottom} id="bottom" />
      <Handle type="source" position={Position.Top} id="top" />
      <Handle type="source" position={Position.Left} id="left" />
      <Handle type="source" position={Position.Right} id="right" />
      <Handle type="source" position={Position.Bottom} id="bottom" />
      {data.label.length > 20
        ? `${data.label.substring(0, 18)}...`
        : data.label}
    </div>
  );
};

const nodeTypes = {
  custom: CustomNode,
};

export function GraphVisualization({
  className,
  initialData,
}: GraphVisualizationProps) {
  const [graphData, setGraphData] = useState<GraphData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [inputMethod, setInputMethod] = useState<"upload" | "paste">("paste");
  const [pastedData, setPastedData] = useState("");
  const [hoveredNodeId, setHoveredNodeId] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Handle initial data from URL
  useEffect(() => {
    if (initialData) {
      try {
        const parsed = parseGraphData(initialData);
        setGraphData(parsed);
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Failed to parse initial data"
        );
      }
    }
  }, [initialData]);

  const parseGraphQLResponse = (jsonString: string): GraphData => {
    try {
      // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- Need to parse dynamic JSON response
      const parsed = JSON.parse(jsonString);

      // Handle direct packageGraph format
      // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access -- GraphQL response structure is dynamic
      if (parsed.packageGraph) {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access -- GraphQL response structure is dynamic
        const packageGraph = parsed.packageGraph;
        // eslint-disable-next-line @typescript-eslint/no-unsafe-argument -- GraphQL response structure is dynamic
        return extractGraphData(packageGraph);
      }

      throw new Error(
        "Invalid GraphQL response format. Expected packageGraph structure."
      );
    } catch (e) {
      throw new Error("Invalid JSON format or unsupported structure");
    }
  };

  const extractGraphData = (packageGraph: {
    nodes?: { items?: Array<{ name: string }> };
    edges?: { items?: Array<{ source: string; target: string }> };
  }): GraphData => {
    const nodes: GraphData["nodes"] = [];
    const edges: GraphData["edges"] = [];

    // Extract nodes from packageGraph.nodes.items

    if (packageGraph.nodes?.items) {
      packageGraph.nodes.items.forEach((item: { name: string }) => {
        if (item.name) {
          nodes.push({
            id: item.name,
            label: item.name,
            name: item.name,
          });
        }
      });
    }

    // Extract edges from packageGraph.edges.items

    if (packageGraph.edges?.items) {
      packageGraph.edges.items.forEach(
        (item: { source: string; target: string }) => {
          if (item.source && item.target) {
            edges.push({
              source: item.source,
              target: item.target,
            });
          }
        }
      );
    }

    return { nodes, edges };
  };

  const parseGraphData = (content: string): GraphData => {
    const trimmed = content.trim();

    // Only support JSON format from turbo query
    if (trimmed.startsWith("{")) {
      return parseGraphQLResponse(trimmed);
    }
    throw new Error('Please provide JSON output from "turbo query" command.');
  };

  const handleFileUpload = (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) return;

    setLoading(true);
    setError(null);

    const reader = new FileReader();
    reader.onload = (e) => {
      try {
        const content = (e.target?.result as string) || "";
        const parsed = parseGraphData(content);
        setGraphData(parsed);
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Failed to parse graph data"
        );
      } finally {
        setLoading(false);
      }
    };
    reader.readAsText(file);
  };

  const handlePasteData = () => {
    if (!pastedData.trim()) return;

    setLoading(true);
    setError(null);

    try {
      const parsed = parseGraphData(pastedData);
      setGraphData(parsed);
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "Failed to parse graph data"
      );
    } finally {
      setLoading(false);
    }
  };

  // Convert graph data to XYFlow format
  const flowData = useMemo(() => {
    if (!graphData) return { nodes: [], edges: [] };

    // Build adjacency lists for topological sorting
    const inDegree = new Map<string, number>();
    const adjacencyList = new Map<string, Array<string>>();

    // Initialize
    graphData.nodes.forEach((node) => {
      inDegree.set(node.id, 0);
      adjacencyList.set(node.id, []);
    });

    // Build the graph
    graphData.edges.forEach((edge) => {
      // In a DAG, edge.source depends on edge.target (target -> source)
      // So we want to show target above source in the hierarchy
      const target = edge.target;
      const source = edge.source;

      if (adjacencyList.has(target) && adjacencyList.has(source)) {
        const targetList = adjacencyList.get(target);
        if (targetList) {
          targetList.push(source);
        }
        inDegree.set(source, (inDegree.get(source) || 0) + 1);
      }
    });

    // Topological sort using Kahn's algorithm
    const queue: Array<string> = [];
    const levels: Array<Array<string>> = [];
    const visited = new Set<string>();

    // Find all nodes with no incoming edges (root nodes)
    graphData.nodes.forEach((node) => {
      if ((inDegree.get(node.id) || 0) === 0) {
        queue.push(node.id);
      }
    });

    // Process nodes level by level
    while (queue.length > 0) {
      const currentLevel: Array<string> = [];
      const nextQueue: Array<string> = [];

      for (const nodeId of queue) {
        if (visited.has(nodeId)) continue;

        visited.add(nodeId);
        currentLevel.push(nodeId);

        // Add neighbors to next level
        const neighbors = adjacencyList.get(nodeId) || [];
        for (const neighbor of neighbors) {
          inDegree.set(neighbor, (inDegree.get(neighbor) || 1) - 1);
          if ((inDegree.get(neighbor) || 0) === 0) {
            nextQueue.push(neighbor);
          }
        }
      }

      if (currentLevel.length > 0) {
        levels.push(currentLevel);
      }
      queue.splice(0, queue.length, ...nextQueue);
    }

    // Add any remaining nodes (shouldn't happen in a valid DAG, but just in case)
    graphData.nodes.forEach((node) => {
      if (!visited.has(node.id)) {
        if (levels.length === 0) levels.push([]);
        levels[levels.length - 1].push(node.id);
      }
    });

    // Calculate node degrees for sizing
    const nodeDegrees = new Map<string, number>();
    graphData.nodes.forEach((node) => {
      nodeDegrees.set(
        node.id,
        graphData.edges.filter(
          (edge) => edge.source === node.id || edge.target === node.id
        ).length
      );
    });

    // Build adjacency lists for hover highlighting
    const directConnections = new Map<string, Set<string>>();
    graphData.nodes.forEach((node) => {
      directConnections.set(node.id, new Set());
    });

    graphData.edges.forEach((edge) => {
      const sourceConnections = directConnections.get(edge.source);
      const targetConnections = directConnections.get(edge.target);
      if (sourceConnections && targetConnections) {
        sourceConnections.add(edge.target);
        targetConnections.add(edge.source);
      }
    });

    // --- Calculate node widths for each node (same as in style) ---
    const nodeWidths = new Map<string, number>();
    graphData.nodes.forEach((node) => {
      const degree = nodeDegrees.get(node.id) || 0;
      nodeWidths.set(node.id, Math.max(70, Math.min(180, 70 + degree * 8)));
    });
    const nodeHeights = new Map<string, number>();
    graphData.nodes.forEach((node) => {
      const degree = nodeDegrees.get(node.id) || 0;
      nodeHeights.set(node.id, Math.max(40, Math.min(60, 40 + degree * 2)));
    });

    // --- Compute x positions for each node in each level to avoid overlap ---
    const levelSpacing = 180;
    const minGap = 32; // Minimum gap between nodes
    const levelNodePositions = new Map<string, { x: number; y: number }>();
    levels.forEach((level, levelIndex) => {
      // Get widths for all nodes in this level
      const widths = level.map((id) => nodeWidths.get(id) || 100);
      const totalWidth =
        widths.reduce((a, b) => a + b, 0) + minGap * (level.length - 1);
      let currentX = -totalWidth / 2;
      level.forEach((id, i) => {
        // Add deterministic jitter for organic look
        const jitter = ((id.charCodeAt(0) + id.length) % 40) - 20;
        const width = widths[i];
        // Add deterministic variation to level spacing for more organic feel
        const levelVariation = ((id.charCodeAt(0) + levelIndex) % 20) - 10;
        const y = levelIndex * levelSpacing + 50 + levelVariation;
        // Center node at currentX + width/2
        levelNodePositions.set(id, { x: currentX + width / 2 + jitter, y });
        currentX += width + minGap;
      });
    });

    const flowNodes = graphData.nodes.map((node) => {
      // Use precomputed position
      const pos = levelNodePositions.get(node.id) || { x: 0, y: 0 };
      const width = nodeWidths.get(node.id) || 100;
      const height = nodeHeights.get(node.id) || 40;
      const isRoot = node.id === "//";

      // Determine if node should be highlighted or faded based on hover state
      let isHighlighted = false;
      let isFaded = false;
      if (hoveredNodeId) {
        const hoveredConnections = directConnections.get(hoveredNodeId);
        if (hoveredConnections) {
          if (node.id === hoveredNodeId) {
            isHighlighted = true;
          } else if (hoveredConnections.has(node.id)) {
            isHighlighted = true;
          } else {
            isFaded = true;
          }
        }
      }

      return {
        id: node.id,
        type: "custom",
        position: pos,
        data: {
          label: node.label,
          isRoot,
          isHighlighted,
          isFaded,
        },
        style: {
          width,
          height,
        },
      };
    });

    // Helper to determine handle side based on direction
    function getHandleDirection(
      from: { x: number; y: number },
      to: { x: number; y: number }
    ) {
      const dx = to.x - from.x;
      const dy = to.y - from.y;
      if (Math.abs(dx) > Math.abs(dy)) {
        // Horizontal is dominant
        return dx > 0 ? "right" : "left";
      }
      // Vertical is dominant
      return dy > 0 ? "bottom" : "top";
    }

    // Create edges with proper styling and handle positions
    const flowEdges = graphData.edges
      .map((edge, index) => {
        const sourcePos = levelNodePositions.get(edge.source);
        const targetPos = levelNodePositions.get(edge.target);
        if (!sourcePos || !targetPos) {
          if (typeof window !== "undefined") {
            // eslint-disable-next-line no-console -- Debug warning for missing nodes
            console.warn("Edge references missing node:", edge);
          }
          return null;
        }

        // Determine if edge should be faded based on hover state
        let isFaded = false;
        if (hoveredNodeId) {
          const hoveredConnections = directConnections.get(hoveredNodeId);
          if (hoveredConnections) {
            // Edge is relevant if either source or target is the hovered node or directly connected
            const isSourceRelevant =
              edge.source === hoveredNodeId ||
              hoveredConnections.has(edge.source);
            const isTargetRelevant =
              edge.target === hoveredNodeId ||
              hoveredConnections.has(edge.target);
            isFaded = !(isSourceRelevant && isTargetRelevant);
          }
        }

        // Determine which handle to use for source and target
        const sourceHandle = getHandleDirection(sourcePos, targetPos);
        const targetHandle = getHandleDirection(targetPos, sourcePos);
        // Calculate edge importance based on node degrees
        const sourceDegree = nodeDegrees.get(edge.source) || 0;
        const targetDegree = nodeDegrees.get(edge.target) || 0;
        const edgeImportance = Math.min(sourceDegree + targetDegree, 8);

        return {
          id: `edge-${index}`,
          source: String(edge.source),
          target: String(edge.target),
          sourceHandle,
          targetHandle,
          type: "default",
          style: {
            stroke: "#3b82f6",
            strokeWidth: Math.max(1, Math.min(4, edgeImportance * 0.3)),
            strokeDasharray: edgeImportance > 4 ? "none" : "3,3", // Solid lines for important connections
            opacity: isFaded ? 0.2 : 0.8,
            transition:
              "opacity 0.2s ease-in-out, stroke-width 0.2s ease-in-out",
          },
        };
      })
      .filter(Boolean);

    return { nodes: flowNodes, edges: flowEdges };
  }, [graphData, hoveredNodeId]);

  const clearGraph = () => {
    setGraphData(null);
    setError(null);
    setPastedData("");
    if (fileInputRef.current) {
      fileInputRef.current.value = "";
    }
  };

  return (
    <div className={className}>
      <Card className="p-6">
        <h3 className="text-lg font-semibold mb-4">
          Upload Package Graph Data
        </h3>

        {/* Input method selector */}
        <div className="flex gap-4 mb-4">
          <Button
            variant={inputMethod === "upload" ? "default" : "outline"}
            onClick={() => {
              setInputMethod("upload");
            }}
          >
            Upload File
          </Button>
          <Button
            variant={inputMethod === "paste" ? "default" : "outline"}
            onClick={() => {
              setInputMethod("paste");
            }}
          >
            Paste Data
          </Button>
        </div>

        {/* File upload */}
        {inputMethod === "upload" && (
          <div className="mb-4">
            <input
              ref={fileInputRef}
              type="file"
              accept=".json"
              onChange={handleFileUpload}
              className="block w-full text-sm text-gray-500 file:mr-4 file:py-2 file:px-4 file:rounded-full file:border-0 file:text-sm file:font-semibold file:bg-blue-50 file:text-blue-700 hover:file:bg-blue-100"
            />
            <p className="text-sm text-gray-500 mt-2">
              Upload a JSON file with the output from <code>turbo query</code>
            </p>
          </div>
        )}

        {/* Paste data */}
        {inputMethod === "paste" && (
          <div className="mb-4">
            <textarea
              value={pastedData}
              onChange={(e) => {
                setPastedData(e.target.value);
              }}
              placeholder='Paste the JSON output from "turbo query" here...'
              className="w-full h-32 p-3 border rounded-md font-mono text-sm"
            />
            <Button
              onClick={handlePasteData}
              disabled={!pastedData.trim() || loading}
              className="mt-2"
            >
              {loading ? "Processing..." : "Visualize Package Graph"}
            </Button>
          </div>
        )}

        {/* Error display */}
        {error && (
          <Callout type="error" className="mb-4">
            {error}
          </Callout>
        )}

        {/* Graph display */}
        {graphData && (
          <div className="border rounded-md p-4 bg-gray-50">
            <div className="flex justify-between items-center mb-4">
              <h4 className="font-semibold">Package Dependency Graph</h4>
              <Button variant="outline" onClick={clearGraph}>
                Clear
              </Button>
            </div>

            <div className="h-[600px] border bg-white rounded">
              <ReactFlow
                nodes={flowData.nodes}
                edges={flowData.edges}
                nodeTypes={nodeTypes}
                fitView
                attributionPosition="bottom-left"
                proOptions={{ hideAttribution: true }}
                onNodeMouseEnter={(_: React.MouseEvent, node: Node) => {
                  setHoveredNodeId(node.id);
                }}
                onNodeMouseLeave={() => {
                  setHoveredNodeId(null);
                }}
              >
                <Controls />
                <Background color="#f1f5f9" gap={16} />
              </ReactFlow>
            </div>

            <div className="mt-4 text-sm text-gray-600">
              <p>
                <strong>Legend:</strong>
              </p>
              <ul className="list-disc ml-4 mt-2">
                <li>
                  <span className="inline-block w-3 h-3 bg-red-500 rounded-full mr-2"></span>
                  Root package (//)
                </li>
                <li>
                  <span className="inline-block w-3 h-3 bg-blue-500 rounded-full mr-2"></span>
                  Packages (size = connection count)
                </li>
                <li>
                  <strong>Layout:</strong> Organic hierarchical layout with
                  centrality-based positioning
                </li>
                <li>
                  <strong>Edges:</strong> Thicker lines = more important
                  connections, dashed = less critical
                </li>
                <li>
                  Nodes are positioned by importance within dependency levels
                </li>
                <li>Hover over nodes to highlight their direct connections</li>
              </ul>
            </div>
          </div>
        )}

        {/* Instructions */}
        {!graphData && (
          <div className="mt-4">
            <h4 className="font-semibold mb-2">
              How to Generate Package Graph Data
            </h4>
            <div className="space-y-2 text-sm">
              <p>Use this command to get package graph data:</p>
              <pre className="bg-gray-100 p-3 rounded text-xs overflow-x-auto">
                <code>{`turbo query '{
  packageGraph {
    nodes {
      items {
        name
      }
    }
    edges {
      items {
        source
        target
      }
    }
  }
}'`}</code>
              </pre>
              <p className="text-gray-600">
                Copy the entire JSON response and paste it above.
              </p>
            </div>
          </div>
        )}
      </Card>
    </div>
  );
}
