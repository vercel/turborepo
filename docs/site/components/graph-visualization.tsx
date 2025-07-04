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
      className={`px-3 py-2 rounded-lg border-2 font-medium text-sm shadow-sm ${
        data.isRoot
          ? "bg-red-500 text-white border-red-600"
          : "bg-blue-500 text-white border-blue-600"
      }`}
    >
      <Handle type="target" position={Position.Top} />
      {data.label.length > 20
        ? `${data.label.substring(0, 18)}...`
        : data.label}
      <Handle type="source" position={Position.Bottom} />
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

    // Create grid layout for better edge visibility
    const nodesPerRow = 6;
    const nodeSpacingX = 220;
    const nodeSpacingY = 120;
    const flowNodes = graphData.nodes.map((node, idx) => {
      const degree = nodeDegrees.get(node.id) || 0;
      const isRoot = node.id === "//";
      return {
        id: node.id,
        type: "custom",
        position: {
          x: (idx % nodesPerRow) * nodeSpacingX,
          y: Math.floor(idx / nodesPerRow) * nodeSpacingY + 100,
        },
        data: {
          label: node.label,
          isRoot,
        },
        style: {
          width: Math.max(80, Math.min(200, 80 + degree * 10)),
        },
      };
    });

    // Edges are generated from the parsed graph data and should match node IDs
    const flowEdges = graphData.edges.map((edge, index) => {
      if (
        !flowNodes.find((n) => n.id === edge.source) ||
        !flowNodes.find((n) => n.id === edge.target)
      ) {
        if (typeof window !== "undefined") {
          // eslint-disable-next-line no-console
          console.warn("Edge references missing node:", edge);
        }
      }

      return {
        id: `edge-${index}`,
        source: String(edge.source),
        target: String(edge.target),
        type: "default", // force default edge type
        style: { stroke: "#ff0000", strokeWidth: 3 }, // bright red
      };
    });

    // Add a hardcoded debug edge between the first two nodes
    if (flowNodes.length > 1) {
      flowEdges.push({
        id: "debug-edge",
        source: flowNodes[0].id,
        target: flowNodes[1].id,
        type: "default",
        style: { stroke: "#00ff00", strokeWidth: 4 }, // bright green
      });
    }

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
                  <strong>Layout:</strong> Automatic hierarchical layout with
                  drag & drop support
                </li>
                <li>Arrows point from dependents to dependencies</li>
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
