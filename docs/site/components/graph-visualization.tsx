"use client";

import { useState, useRef, useEffect } from "react";
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

export function GraphVisualization({ className }: GraphVisualizationProps) {
  const [graphData, setGraphData] = useState<GraphData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [inputMethod, setInputMethod] = useState<"upload" | "paste">("paste");
  const [pastedData, setPastedData] = useState("");
  const fileInputRef = useRef<HTMLInputElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);

  const parseGraphQLResponse = (jsonString: string): GraphData => {
    try {
      // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
      const parsed = JSON.parse(jsonString);

      // Handle turbo query GraphQL response format
      // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access
      if (parsed.data?.packageGraph) {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access
        const packageGraph = parsed.data.packageGraph;
        const nodes: GraphData["nodes"] = [];
        const edges: GraphData["edges"] = [];

        // Extract nodes from packageGraph.nodes.items
        // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access
        if (packageGraph.nodes?.items) {
          // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access
          packageGraph.nodes.items.forEach((item: any) => {
            // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access
            if (item.name) {
              nodes.push({
                // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access
                id: item.name,
                // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access
                label: item.name,
                // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access
                name: item.name,
              });
            }
          });
        }

        // Extract edges from packageGraph.edges.items
        // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access
        if (packageGraph.edges?.items) {
          // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access
          packageGraph.edges.items.forEach((item: any) => {
            // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access
            if (item.source && item.target) {
              edges.push({
                // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access
                source: item.source,
                // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access
                target: item.target,
              });
            }
          });
        }

        return { nodes, edges };
      }

      // Handle direct format (for backwards compatibility)
      // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access
      if (parsed.nodes && parsed.edges) {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-return
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

  const renderGraph = () => {
    if (!graphData || !svgRef.current) return;

    const svg = svgRef.current;
    const width = 800;
    const height = 600;

    // Clear previous content
    svg.innerHTML = "";

    // Simple force-directed layout simulation
    const nodes = graphData.nodes.map((node) => ({
      ...node,
      x: Math.random() * (width - 100) + 50,
      y: Math.random() * (height - 100) + 50,
      vx: 0,
      vy: 0,
    }));

    const edges = graphData.edges
      .map((edge) => ({
        ...edge,
        source: nodes.find((n) => n.id === edge.source),
        target: nodes.find((n) => n.id === edge.target),
      }))
      .filter((e) => e.source && e.target);

    // Simple physics simulation
    const simulate = () => {
      const alpha = 0.1;
      const linkDistance = 100;
      const chargeStrength = -300;

      // Apply forces
      for (let i = 0; i < 50; i++) {
        // Link force
        edges.forEach((edge) => {
          if (!edge.source || !edge.target) return;

          const dx = edge.target.x - edge.source.x;
          const dy = edge.target.y - edge.source.y;
          const distance = Math.sqrt(dx * dx + dy * dy) || 1;
          const force = (distance - linkDistance) * alpha;

          const fx = (dx / distance) * force;
          const fy = (dy / distance) * force;

          edge.source.vx += fx;
          edge.source.vy += fy;
          edge.target.vx -= fx;
          edge.target.vy -= fy;
        });

        // Charge force
        nodes.forEach((nodeA, i) => {
          nodes.forEach((nodeB, j) => {
            if (i === j) return;

            const dx = nodeB.x - nodeA.x;
            const dy = nodeB.y - nodeA.y;
            const distance = Math.sqrt(dx * dx + dy * dy) || 1;
            const force = chargeStrength / (distance * distance);

            const fx = (dx / distance) * force;
            const fy = (dy / distance) * force;

            nodeA.vx -= fx;
            nodeA.vy -= fy;
          });
        });

        // Update positions
        nodes.forEach((node) => {
          node.x += node.vx * alpha;
          node.y += node.vy * alpha;
          node.vx *= 0.9;
          node.vy *= 0.9;

          // Keep within bounds
          node.x = Math.max(50, Math.min(width - 50, node.x));
          node.y = Math.max(50, Math.min(height - 50, node.y));
        });
      }
    };

    simulate();

    // Render edges
    edges.forEach((edge) => {
      if (!edge.source || !edge.target) return;

      const line = document.createElementNS(
        "http://www.w3.org/2000/svg",
        "line"
      );
      line.setAttribute("x1", edge.source.x.toString());
      line.setAttribute("y1", edge.source.y.toString());
      line.setAttribute("x2", edge.target.x.toString());
      line.setAttribute("y2", edge.target.y.toString());
      line.setAttribute("stroke", "#6b7280");
      line.setAttribute("stroke-width", "2");
      line.setAttribute("marker-end", "url(#arrowhead)");
      svg.appendChild(line);
    });

    // Render nodes
    nodes.forEach((node) => {
      const group = document.createElementNS("http://www.w3.org/2000/svg", "g");

      // Node circle
      const circle = document.createElementNS(
        "http://www.w3.org/2000/svg",
        "circle"
      );
      circle.setAttribute("cx", node.x.toString());
      circle.setAttribute("cy", node.y.toString());
      circle.setAttribute("r", "20");
      circle.setAttribute("fill", node.id === "//" ? "#ef4444" : "#3b82f6");
      circle.setAttribute("stroke", "#1f2937");
      circle.setAttribute("stroke-width", "2");
      group.appendChild(circle);

      // Node label
      const text = document.createElementNS(
        "http://www.w3.org/2000/svg",
        "text"
      );
      text.setAttribute("x", node.x.toString());
      text.setAttribute("y", (node.y + 35).toString());
      text.setAttribute("text-anchor", "middle");
      text.setAttribute("font-size", "12");
      text.setAttribute("fill", "#1f2937");
      text.textContent =
        node.label.length > 15
          ? `${node.label.substring(0, 15)}...`
          : node.label;
      group.appendChild(text);

      svg.appendChild(group);
    });

    // Add arrow marker definition
    const defs = document.createElementNS("http://www.w3.org/2000/svg", "defs");
    const marker = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "marker"
    );
    marker.setAttribute("id", "arrowhead");
    marker.setAttribute("markerWidth", "10");
    marker.setAttribute("markerHeight", "7");
    marker.setAttribute("refX", "10");
    marker.setAttribute("refY", "3.5");
    marker.setAttribute("orient", "auto");

    const polygon = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "polygon"
    );
    polygon.setAttribute("points", "0 0, 10 3.5, 0 7");
    polygon.setAttribute("fill", "#6b7280");
    marker.appendChild(polygon);
    defs.appendChild(marker);
    svg.appendChild(defs);
  };

  useEffect(() => {
    if (graphData) {
      renderGraph();
    }
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

            <div className="overflow-auto">
              <svg
                ref={svgRef}
                width="800"
                height="600"
                viewBox="0 0 800 600"
                className="border bg-white rounded"
              />
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
                  Packages
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
