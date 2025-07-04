"use client";

import { useState, useRef, useMemo } from "react";
import {
  ReactFlow,
  Controls,
  Background,
  Handle,
  Position,
} from "@xyflow/react";
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
}

// Custom node component for better styling
const CustomNode = ({
  data,
}: {
  data: { label: string; isRoot?: boolean };
}) => {
  return (
    <div
      className={`relative px-3 py-2 rounded-lg border-2 font-medium text-sm shadow-sm ${
        data.isRoot
          ? "bg-red-500 text-white border-red-600"
          : "bg-blue-500 text-white border-blue-600"
      }`}
      style={{ minWidth: 80, minHeight: 40 }}
    >
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

export function GraphVisualization({ className }: GraphVisualizationProps) {
  const [graphData, setGraphData] = useState<GraphData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [inputMethod, setInputMethod] = useState<"upload" | "paste">("paste");
  const [pastedData, setPastedData] = useState("");
  const fileInputRef = useRef<HTMLInputElement>(null);

  const parseGraphQLResponse = (jsonString: string): GraphData => {
    try {
      // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- Need to parse dynamic JSON response
      const parsed = JSON.parse(jsonString);

      // Handle turbo query GraphQL response format
      // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access -- GraphQL response structure is dynamic
      if (parsed.data?.packageGraph) {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access -- GraphQL response structure is dynamic
        const packageGraph = parsed.data.packageGraph;
        const nodes: GraphData["nodes"] = [];
        const edges: GraphData["edges"] = [];

        // Extract nodes from packageGraph.nodes.items
        // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access -- GraphQL response structure is dynamic
        if (packageGraph.nodes?.items) {
          // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access -- GraphQL response structure is dynamic
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
        // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access -- GraphQL response structure is dynamic
        if (packageGraph.edges?.items) {
          // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access -- GraphQL response structure is dynamic
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
      }

      // Handle direct format (for backwards compatibility)
      // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access -- Backwards compatibility with unknown structure
      if (parsed.nodes && parsed.edges) {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-return -- Backwards compatibility with unknown structure
        return parsed;
      }

      throw new Error(
        "Invalid GraphQL response format. Expected data.packageGraph structure."
      );
    } catch (e) {
      throw new Error("Invalid JSON format or unsupported structure");
    }
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

    // Position nodes in hierarchical layout
    const nodeSpacingX = 200;
    const levelSpacing = 200;

    // Map nodeId to position for edge direction calculation
    const nodePositions = new Map<string, { x: number; y: number }>();

    const flowNodes = graphData.nodes.map((node) => {
      const degree = nodeDegrees.get(node.id) || 0;
      const isRoot = node.id === "//";

      // Find which level this node belongs to
      let levelIndex = 0;
      let positionInLevel = 0;

      for (let i = 0; i < levels.length; i++) {
        const level = levels[i];
        const nodeIndex = level.indexOf(node.id);
        if (nodeIndex !== -1) {
          levelIndex = i;
          positionInLevel = nodeIndex;
          break;
        }
      }

      // Calculate position
      const x = positionInLevel * nodeSpacingX + 100;
      const y = levelIndex * levelSpacing + 100;

      // Store for edge direction
      nodePositions.set(node.id, { x, y });

      return {
        id: node.id,
        type: "custom",
        position: { x, y },
        data: {
          label: node.label,
          isRoot,
        },
        style: {
          width: Math.max(80, Math.min(200, 80 + degree * 10)),
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
        const sourcePos = nodePositions.get(edge.source);
        const targetPos = nodePositions.get(edge.target);
        if (!sourcePos || !targetPos) {
          if (typeof window !== "undefined") {
            // eslint-disable-next-line no-console -- Debug warning for missing nodes
            console.warn("Edge references missing node:", edge);
          }
          return null;
        }
        // Determine which handle to use for source and target
        const sourceHandle = getHandleDirection(sourcePos, targetPos);
        const targetHandle = getHandleDirection(targetPos, sourcePos);
        return {
          id: `edge-${index}`,
          source: String(edge.source),
          target: String(edge.target),
          sourceHandle,
          targetHandle,
          type: "default",
          style: {
            stroke: "#3b82f6",
            strokeWidth: 2,
            strokeDasharray: "5,5", // Dashed line to show dependency direction
          },
          animated: true, // Animate the flow to show direction
        };
      })
      .filter(Boolean);

    return { nodes: flowNodes, edges: flowEdges };
  }, [graphData]);

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
                  <strong>Layout:</strong> Hierarchical DAG layout -
                  dependencies at top, dependents below
                </li>
                <li>
                  Animated arrows show dependency direction (dependents →
                  dependencies)
                </li>
                <li>
                  Nodes are arranged in levels based on their dependency depth
                </li>
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
