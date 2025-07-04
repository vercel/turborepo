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

  const renderGraph = () => {
    if (!graphData || !svgRef.current) return;

    const svg = svgRef.current;
    const width = 1000;
    const height = 800;

    // Clear previous content
    svg.innerHTML = "";

    // Enhanced hierarchical layout algorithm
    const nodes = graphData.nodes.map((node) => ({
      ...node,
      x: 0,
      y: 0,
      vx: 0,
      vy: 0,
      degree: 0,
      layer: 0,
      dependencyDepth: 0,
    }));

    // Calculate node degrees and dependency relationships
    nodes.forEach((node) => {
      node.degree = graphData.edges.filter(
        (edge) => edge.source === node.id || edge.target === node.id
      ).length;
    });

    // Calculate dependency depth for hierarchical layering
    const calculateDependencyDepth = () => {
      const dependencyMap = new Map<string, Array<string>>();
      const dependentMap = new Map<string, Array<string>>();

      // Build dependency and dependent maps
      nodes.forEach((node) => {
        dependencyMap.set(node.id, []);
        dependentMap.set(node.id, []);
      });

      graphData.edges.forEach((edge) => {
        // In our graph, arrow points from dependent to dependency
        dependencyMap.get(edge.source)?.push(edge.target);
        dependentMap.get(edge.target)?.push(edge.source);
      });

      // Calculate depth using topological approach
      const visited = new Set<string>();
      const depths = new Map<string, number>();

      const calculateDepth = (nodeId: string): number => {
        const existingDepth = depths.get(nodeId);
        if (existingDepth !== undefined) return existingDepth;
        if (visited.has(nodeId)) return 0; // Cycle detection

        visited.add(nodeId);
        const dependencies = dependencyMap.get(nodeId) || [];

        if (dependencies.length === 0) {
          depths.set(nodeId, 0);
          visited.delete(nodeId);
          return 0;
        }

        const maxDepth = Math.max(
          ...dependencies.map((dep) => calculateDepth(dep))
        );
        const newDepth = maxDepth + 1;
        depths.set(nodeId, newDepth);
        visited.delete(nodeId);
        return newDepth;
      };

      // Calculate depth for all nodes
      nodes.forEach((node) => {
        node.dependencyDepth = calculateDepth(node.id);
      });

      return Math.max(...nodes.map((n) => n.dependencyDepth));
    };

    const maxDepth = calculateDependencyDepth();

    // Group nodes into layers based on dependency depth
    const layers: Array<Array<(typeof nodes)[0]>> = [];
    for (let i = 0; i <= maxDepth; i++) {
      layers[i] = nodes.filter((node) => node.dependencyDepth === i);
    }

    // Position nodes in hierarchical layers
    const layerHeight =
      layers.length > 1 ? (height - 160) / (layers.length - 1) : 0;
    const padding = 80;

    layers.forEach((layer, layerIndex) => {
      const y = padding + layerIndex * layerHeight;
      const layerWidth = width - 2 * padding;
      const nodeSpacing =
        layer.length > 1 ? layerWidth / (layer.length - 1) : 0;

      layer.forEach((node, nodeIndex) => {
        node.layer = layerIndex;
        if (layer.length === 1) {
          node.x = width / 2;
        } else {
          node.x = padding + nodeIndex * nodeSpacing;
        }
        node.y = y;

        // Add small random offset to prevent perfect alignment
        node.x += (Math.random() - 0.5) * 20;
        node.y += (Math.random() - 0.5) * 15;
      });
    });

    const edges = graphData.edges
      .map((edge) => ({
        ...edge,
        source: nodes.find((n) => n.id === edge.source),
        target: nodes.find((n) => n.id === edge.target),
      }))
      .filter((e) => e.source && e.target);

    // Hierarchical physics simulation with constrained movement
    const simulate = () => {
      const iterations = 100;
      const horizontalRepulsion = -400;
      const verticalConstraint = 0.3; // Strength of layer constraint

      for (let i = 0; i < iterations; i++) {
        const alpha = Math.max(0.02, 1 - i / iterations);

        // Reset forces
        nodes.forEach((node) => {
          node.vx = 0;
          node.vy = 0;
        });

        // Link forces - attract connected nodes but respect hierarchy
        edges.forEach((edge) => {
          if (!edge.source || !edge.target) return;

          const dx = edge.target.x - edge.source.x;
          const dy = edge.target.y - edge.source.y;

          // Horizontal attraction
          const horizontalForce = dx * alpha * 0.05;
          edge.source.vx += horizontalForce;
          edge.target.vx -= horizontalForce;

          // Gentle vertical attraction (less strong to maintain layers)
          const verticalForce = dy * alpha * 0.02;
          edge.source.vy += verticalForce;
          edge.target.vy -= verticalForce;
        });

        // Horizontal repulsion within layers (prevent overlap)
        nodes.forEach((nodeA, idx) => {
          nodes.forEach((nodeB, jdx) => {
            if (idx >= jdx) return;

            // Stronger repulsion for nodes in the same layer
            const sameLayer = nodeA.layer === nodeB.layer;
            const dx = nodeB.x - nodeA.x;
            const dy = nodeB.y - nodeA.y;
            const distance = Math.sqrt(dx * dx + dy * dy) || 0.1;

            if (sameLayer && Math.abs(dx) < 200) {
              const force =
                (horizontalRepulsion * alpha) / (distance * distance);
              const fx = (dx / distance) * force;

              nodeA.vx -= fx;
              nodeB.vx += fx;
            }

            // Prevent vertical overlap between layers
            if (Math.abs(dy) < 60 && Math.abs(dx) < 100) {
              const force = (-200 * alpha) / (distance * distance);
              const fy = (dy / distance) * force;

              nodeA.vy -= fy;
              nodeB.vy += fy;
            }
          });
        });

        // Layer constraint - pull nodes back to their designated layer
        layers.forEach((layer, layerIndex) => {
          const targetY = padding + layerIndex * layerHeight;
          layer.forEach((node) => {
            const dy = targetY - node.y;
            node.vy += dy * verticalConstraint * alpha;
          });
        });

        // Update positions with constrained movement
        nodes.forEach((node) => {
          // Stronger horizontal damping, gentler vertical damping
          node.vx *= 0.8;
          node.vy *= 0.9;

          node.x += node.vx;
          node.y += node.vy;

          // Keep within bounds
          const boundaryPadding = 60;
          node.x = Math.max(
            boundaryPadding,
            Math.min(width - boundaryPadding, node.x)
          );

          // Constrain vertical movement to stay near layer
          const targetY = padding + node.layer * layerHeight;
          const maxVerticalDeviation = 40;
          node.y = Math.max(
            targetY - maxVerticalDeviation,
            Math.min(targetY + maxVerticalDeviation, node.y)
          );
        });
      }

      // Final horizontal spacing adjustment within layers
      layers.forEach((layer) => {
        if (layer.length <= 1) return;

        // Sort by current x position
        layer.sort((a, b) => a.x - b.x);

        // Adjust spacing to prevent overlaps
        const minSpacing = 110;
        for (let i = 1; i < layer.length; i++) {
          const prev = layer[i - 1];
          const curr = layer[i];
          const distance = curr.x - prev.x;

          if (distance < minSpacing) {
            const adjustment = (minSpacing - distance) / 2;
            prev.x -= adjustment;
            curr.x += adjustment;

            // Keep within bounds
            const boundaryPadding = 60;
            prev.x = Math.max(boundaryPadding, prev.x);
            curr.x = Math.min(width - boundaryPadding, curr.x);
          }
        }
      });
    };

    simulate();

    // Render nodes
    nodes.forEach((node) => {
      const group = document.createElementNS("http://www.w3.org/2000/svg", "g");
      group.setAttribute("data-node-id", node.id);
      group.classList.add("node-group");

      // Node circle with size based on degree
      const nodeRadius = Math.max(
        15,
        Math.min(25, 15 + (node.degree || 0) * 1.5)
      );
      const circle = document.createElementNS(
        "http://www.w3.org/2000/svg",
        "circle"
      );
      circle.setAttribute("cx", node.x.toString());
      circle.setAttribute("cy", node.y.toString());
      circle.setAttribute("r", nodeRadius.toString());
      circle.setAttribute("fill", node.id === "//" ? "#ef4444" : "#3b82f6");
      circle.setAttribute("stroke", "#1f2937");
      circle.setAttribute("stroke-width", "2");
      circle.setAttribute("opacity", "0.9");
      circle.classList.add("node-circle");
      group.appendChild(circle);

      // Background for text to improve readability
      const textContent =
        node.label.length > 20
          ? `${node.label.substring(0, 18)}...`
          : node.label;
      const textWidth = textContent.length * 6.5;
      const textBg = document.createElementNS(
        "http://www.w3.org/2000/svg",
        "rect"
      );
      textBg.setAttribute("x", (node.x - textWidth / 2 - 2).toString());
      textBg.setAttribute("y", (node.y + nodeRadius + 8).toString());
      textBg.setAttribute("width", (textWidth + 4).toString());
      textBg.setAttribute("height", "16");
      textBg.setAttribute("fill", "rgba(255, 255, 255, 0.9)");
      textBg.setAttribute("rx", "2");
      textBg.classList.add("node-text-bg");
      group.appendChild(textBg);

      // Node label
      const text = document.createElementNS(
        "http://www.w3.org/2000/svg",
        "text"
      );
      text.setAttribute("x", node.x.toString());
      text.setAttribute("y", (node.y + nodeRadius + 18).toString());
      text.setAttribute("text-anchor", "middle");
      text.setAttribute("font-size", "11");
      text.setAttribute("font-family", "system-ui, -apple-system, sans-serif");
      text.setAttribute("fill", "#1f2937");
      text.setAttribute("font-weight", "500");
      text.textContent = textContent;
      text.classList.add("node-text");
      group.appendChild(text);

      // Add hover interaction
      const findConnectedNodes = (
        nodeId: string,
        visited = new Set<string>()
      ): Set<string> => {
        if (visited.has(nodeId)) return visited;
        visited.add(nodeId);

        // Find all directly connected nodes (both dependencies and dependents)
        edges.forEach((edge) => {
          if (edge.source?.id === nodeId && edge.target) {
            findConnectedNodes(edge.target.id, visited);
          }
          if (edge.target?.id === nodeId && edge.source) {
            findConnectedNodes(edge.source.id, visited);
          }
        });

        return visited;
      };

      const highlightConnectedNodes = () => {
        const connectedNodes = findConnectedNodes(node.id);

        // Fade out unconnected nodes and their edges
        svg.querySelectorAll(".node-group").forEach((g: Element) => {
          const nodeId = g.getAttribute("data-node-id");
          if (nodeId && !connectedNodes.has(nodeId)) {
            g.classList.add("faded");
          }
        });

        edges.forEach((edge, index) => {
          const line = svg.querySelector(`[data-edge-index="${index}"]`);
          if (!line) return;

          const isConnected =
            edge.source &&
            connectedNodes.has(edge.source.id) &&
            edge.target &&
            connectedNodes.has(edge.target.id);

          if (!isConnected) {
            line.classList.add("faded");
          }
        });
      };

      const resetHighlight = () => {
        svg.querySelectorAll(".faded").forEach((el) => {
          el.classList.remove("faded");
        });
      };

      group.addEventListener("mouseenter", highlightConnectedNodes);
      group.addEventListener("mouseleave", resetHighlight);

      svg.appendChild(group);
    });

    // Add styles for hover effects
    const style = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "style"
    );
    style.textContent = `
      .node-group { transition: opacity 0.2s; }
      .node-group.faded { opacity: 0.2; }
      line { transition: opacity 0.2s; }
      line.faded { opacity: 0.1; }
    `;
    svg.appendChild(style);

    // Render edges (moved after nodes to be on top)
    edges.forEach((edge, index) => {
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
      line.setAttribute("data-edge-index", index.toString());
      svg.appendChild(line);
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
                width="1000"
                height="800"
                viewBox="0 0 1000 800"
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
                  Packages (size = connection count)
                </li>
                <li>
                  <strong>Layout:</strong> Top layers = independent packages,
                  bottom layers = dependent packages
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
