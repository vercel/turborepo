"use client";

import { useSearchParams } from "next/navigation";
import {
  useEffect,
  useState,
  useRef,
  useCallback,
  Suspense,
  useMemo,
} from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  useReactFlow,
  type Node,
  type Edge,
  type NodeMouseHandler,
} from "reactflow";
import ELK from "elkjs/lib/elk.bundled.js";
import { Package } from "lucide-react";
import { DynamicCodeBlock } from "fumadocs-ui/components/dynamic-codeblock";
import { createCssVariablesTheme } from "shiki";

import "reactflow/dist/base.css";
import "./turbo-flow.css";

import { Callout } from "#components/callout.tsx";
import TurboNode, { type TurboNodeData } from "./turbo-node";
import TurboEdge from "./turbo-edge";
import FunctionIcon from "./function-icon";

const theme = createCssVariablesTheme({
  name: "css-variables",
  variablePrefix: "--shiki-",
  variableDefaults: {},
});

// Types matching Rust server
interface PackageNode {
  id: string;
  name: string;
  path: string;
  scripts: Array<string>;
  isRoot: boolean;
}

interface TaskNode {
  id: string;
  package: string;
  task: string;
  script: string;
}

interface GraphEdge {
  source: string;
  target: string;
}

interface PackageGraphData {
  nodes: Array<PackageNode>;
  edges: Array<GraphEdge>;
}

interface TaskGraphData {
  nodes: Array<TaskNode>;
  edges: Array<GraphEdge>;
}

interface GraphState {
  packageGraph: PackageGraphData;
  taskGraph: TaskGraphData;
  repoRoot: string;
  turboVersion: string;
}

interface ServerMessage {
  type: "init" | "update" | "ping" | "error";
  data?: GraphState;
  message?: string;
}

type GraphView = "packages" | "tasks";

// Selection mode: none -> direct (first click) -> blocks (second click) -> dependsOn (third click) -> none (fourth click)
type SelectionMode = "none" | "direct" | "blocks" | "dependsOn";

const elk = new ELK();

// Turbo node and edge types
const nodeTypes = {
  turbo: TurboNode,
};

const edgeTypes = {
  turbo: TurboEdge,
};

const defaultEdgeOptions = {
  type: "turbo",
};

// Constants for node sizing
const NODE_HEIGHT = 70;
const NODE_PADDING_X = 60; // Padding for icon, margins, and handle areas
const MIN_NODE_WIDTH = 150;
const CHAR_WIDTH = 9.6; // Approximate character width for "Fira Mono" at 16px
const SUBTITLE_CHAR_WIDTH = 7.2; // Approximate character width at 12px
const NODE_SPACING = 50; // Consistent spacing between nodes

// Calculate node width based on content
function calculateNodeWidth(data: TurboNodeData): number {
  const titleWidth = data.title.length * CHAR_WIDTH;
  const subtitleWidth = (data.subtitle?.length ?? 0) * SUBTITLE_CHAR_WIDTH;
  const contentWidth = Math.max(titleWidth, subtitleWidth);
  return Math.max(MIN_NODE_WIDTH, contentWidth + NODE_PADDING_X);
}

// ELK layout function
async function getLayoutedElements(
  nodes: Array<Node<TurboNodeData>>,
  edges: Array<Edge>
): Promise<{ nodes: Array<Node>; edges: Array<Edge> }> {
  if (nodes.length === 0) {
    return { nodes: [], edges: [] };
  }

  // Calculate width for each node based on its content
  const nodeWidths = new Map<string, number>();
  for (const node of nodes) {
    nodeWidths.set(node.id, calculateNodeWidth(node.data));
  }

  const graph = {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "DOWN",
      "elk.spacing.nodeNode": String(NODE_SPACING),
      "elk.layered.spacing.nodeNodeBetweenLayers": "150",
      "elk.spacing.componentComponent": "150",
      "elk.layered.spacing.edgeNodeBetweenLayers": "50",
      "elk.layered.nodePlacement.strategy": "NETWORK_SIMPLEX",
    },
    children: nodes.map((node) => ({
      id: node.id,
      width: nodeWidths.get(node.id) ?? MIN_NODE_WIDTH,
      height: NODE_HEIGHT,
    })),
    edges: edges.map((edge, i) => ({
      id: `e${i}`,
      sources: [edge.source],
      targets: [edge.target],
    })),
  };

  const layoutedGraph = await elk.layout(graph);

  return {
    nodes: nodes.map((node) => {
      const layoutedNode = layoutedGraph.children?.find(
        (n: { id: string; x?: number; y?: number }) => n.id === node.id
      );
      return {
        ...node,
        position: { x: layoutedNode?.x ?? 0, y: layoutedNode?.y ?? 0 },
      };
    }),
    edges,
  };
}

// Get direct dependencies (nodes directly connected to the selected node)
function getDirectDependencies(
  nodeId: string,
  edges: Array<GraphEdge>
): Set<string> {
  const connected = new Set<string>();
  connected.add(nodeId);

  for (const edge of edges) {
    if (edge.source === nodeId) {
      connected.add(edge.target);
    }
    if (edge.target === nodeId) {
      connected.add(edge.source);
    }
  }

  return connected;
}

// Get affected nodes (packages/tasks whose hash would change if the selected node changes)
// If package A changes, then all packages that depend on A (directly or transitively) are affected.
// In the edge model: edge.source depends on edge.target (arrow points from dependent to dependency)
// So we traverse "upstream" - following edges backwards from target to source
function getAffectedNodes(
  nodeId: string,
  edges: Array<GraphEdge>
): Set<string> {
  const affected = new Set<string>();
  affected.add(nodeId);

  // Build an adjacency list for reverse traversal (dependency -> dependents)
  const dependentsMap = new Map<string, Array<string>>();
  for (const edge of edges) {
    // edge.source depends on edge.target
    // So edge.target has edge.source as a dependent
    const dependents = dependentsMap.get(edge.target) || [];
    dependents.push(edge.source);
    dependentsMap.set(edge.target, dependents);
  }

  // BFS to find all transitively affected nodes
  const queue = [nodeId];
  while (queue.length > 0) {
    const current = queue.shift()!;
    const dependents = dependentsMap.get(current) || [];

    for (const dependent of dependents) {
      if (!affected.has(dependent)) {
        affected.add(dependent);
        queue.push(dependent);
      }
    }
  }

  return affected;
}

// Get nodes that affect the selected node (transitive dependencies)
// These are the packages that, if changed, would cause the selected node's hash to change.
// In the edge model: edge.source depends on edge.target
// So we traverse "downstream" - following edges from source to target
function getAffectsNodes(nodeId: string, edges: Array<GraphEdge>): Set<string> {
  const affects = new Set<string>();
  affects.add(nodeId);

  // Build an adjacency list for forward traversal (dependent -> dependencies)
  const dependenciesMap = new Map<string, Array<string>>();
  for (const edge of edges) {
    // edge.source depends on edge.target
    const dependencies = dependenciesMap.get(edge.source) || [];
    dependencies.push(edge.target);
    dependenciesMap.set(edge.source, dependencies);
  }

  // BFS to find all transitive dependencies
  const queue = [nodeId];
  while (queue.length > 0) {
    const current = queue.shift()!;
    const dependencies = dependenciesMap.get(current) || [];

    for (const dependency of dependencies) {
      if (!affects.has(dependency)) {
        affects.add(dependency);
        queue.push(dependency);
      }
    }
  }

  return affects;
}

// Get edges that connect the visible nodes
function getConnectedEdges(
  visibleNodes: Set<string>,
  edges: Array<GraphEdge>
): Set<string> {
  const connectedEdges = new Set<string>();

  edges.forEach((edge, i) => {
    if (visibleNodes.has(edge.source) && visibleNodes.has(edge.target)) {
      connectedEdges.add(`e${i}`);
    }
  });

  return connectedEdges;
}

function SetupInstructions() {
  return (
    <div
      className="flex items-center justify-center min-h-screen"
      style={{ backgroundColor: "var(--ds-background-100)" }}
    >
      <div
        className="max-w-md p-8 rounded-lg shadow-[10px_0_15px_rgba(42,138,246,0.2),-10px_0_15px_rgba(233,42,103,0.2)]"
        style={{
          backgroundColor: "var(--ds-background-200)",
          border: "1px solid var(--ds-gray-400)",
        }}
      >
        <h1
          className="text-2xl font-bold mb-4"
          style={{ color: "var(--ds-gray-1000)" }}
        >
          Turborepo Devtools
        </h1>
        <p className="mb-4" style={{ color: "var(--ds-gray-900)" }}>
          Run the following command in your Turborepo to start the devtools
          server:
        </p>
        <DynamicCodeBlock
          lang="bash"
          code="turbo devtools"
          options={
            {
              themes: {
                light: theme,
                dark: theme,
              },
            } as Parameters<typeof DynamicCodeBlock>[0]["options"]
          }
        />
        <p
          className="text-sm mt-4 mb-4"
          style={{ color: "var(--ds-gray-900)" }}
        >
          This will automatically open this page with the correct connection
          parameters.
        </p>
        <Callout type="info">
          <p>
            Already ran it? Add{" "}
            <code
              className="px-1.5 py-0.5 rounded text-xs"
              style={{
                backgroundColor: "var(--ds-gray-200)",
                color: "var(--ds-gray-1000)",
              }}
            >
              ?port=&lt;your-port&gt;
            </code>{" "}
            to the URL to get connected.
          </p>
        </Callout>
      </div>
    </div>
  );
}

function DisconnectedOverlay({ port }: { port: string }) {
  return (
    <div className="absolute inset-0 z-10 bg-black/50 dark:bg-black/70 flex items-center justify-center">
      <div
        className="p-6 rounded-lg shadow-[10px_0_15px_rgba(42,138,246,0.2),-10px_0_15px_rgba(233,42,103,0.2)] max-w-md text-center"
        style={{ backgroundColor: "var(--ds-background-200)" }}
      >
        <h2
          className="text-xl font-semibold mb-2"
          style={{ color: "var(--ds-gray-1000)" }}
        >
          Disconnected
        </h2>
        <p className="mb-4" style={{ color: "var(--ds-gray-900)" }}>
          The connection to turbo devtools was lost. Run the command below to
          reconnect:
        </p>
        <pre
          className="p-3 rounded-md text-sm border border-[#95679e]"
          style={{
            backgroundColor: "var(--ds-background-100)",
            color: "var(--ds-gray-1000)",
          }}
        >
          turbo devtools --port {port}
        </pre>
      </div>
    </div>
  );
}

function ConnectionStatus({ isConnected }: { isConnected: boolean }) {
  return (
    <div className="flex items-center gap-2">
      <div
        className={`w-2 h-2 rounded-full ${
          isConnected ? "bg-[#2a8af6]" : "bg-[#e92a67]"
        }`}
      />
      <span className="text-sm" style={{ color: "var(--ds-gray-900)" }}>
        {isConnected ? "Connected" : "Disconnected"}
      </span>
    </div>
  );
}

function GraphViewToggle({
  view,
  onViewChange,
}: {
  view: GraphView;
  onViewChange: (view: GraphView) => void;
}) {
  return (
    <div
      className="flex gap-1 rounded-lg p-1"
      style={{
        backgroundColor: "var(--ds-gray-200)",
        border: "1px solid var(--ds-gray-400)",
      }}
    >
      <button
        onClick={() => {
          onViewChange("packages");
        }}
        className="flex-1 py-1 text-sm rounded-md transition-colors"
        style={{
          color:
            view === "packages" ? "var(--ds-gray-1000)" : "var(--ds-gray-900)",
          backgroundColor:
            view === "packages" ? "var(--ds-background-100)" : "transparent",
        }}
      >
        Packages
      </button>
      <button
        onClick={() => {
          onViewChange("tasks");
        }}
        className="flex-1 py-1 text-sm rounded-md transition-colors"
        style={{
          color:
            view === "tasks" ? "var(--ds-gray-1000)" : "var(--ds-gray-900)",
          backgroundColor:
            view === "tasks" ? "var(--ds-background-100)" : "transparent",
        }}
      >
        Tasks
      </button>
    </div>
  );
}

type ActiveSelectionMode = Exclude<SelectionMode, "none">;

function getModeOptions(view: GraphView): Array<{
  mode: ActiveSelectionMode;
  getLabel: () => { prefix: string; suffix: string };
}> {
  if (view === "tasks") {
    return [
      {
        mode: "direct",
        getLabel: () => ({ prefix: "Direct neighbors of", suffix: "" }),
      },
      {
        mode: "blocks",
        getLabel: () => ({ prefix: "Blocked by", suffix: "" }),
      },
      {
        mode: "dependsOn",
        getLabel: () => ({ prefix: "", suffix: "depends on..." }),
      },
    ];
  }
  // Package graph
  return [
    {
      mode: "direct",
      getLabel: () => ({ prefix: "Direct neighbors of", suffix: "" }),
    },
    {
      mode: "blocks",
      getLabel: () => ({ prefix: "", suffix: "affects..." }),
    },
    {
      mode: "dependsOn",
      getLabel: () => ({ prefix: "Packages that affect", suffix: "" }),
    },
  ];
}

function SelectionIndicator({
  selectedNode,
  selectionMode,
  view,
  isOpen,
  onToggleOpen,
  onModeChange,
  onClear,
}: {
  selectedNode: string | null;
  selectionMode: SelectionMode;
  view: GraphView;
  isOpen: boolean;
  onToggleOpen: () => void;
  onModeChange: (mode: ActiveSelectionMode) => void;
  onClear: () => void;
}) {
  if (!selectedNode || selectionMode === "none") return null;

  const modeOptions = getModeOptions(view);
  const currentOption = modeOptions.find((opt) => opt.mode === selectionMode);
  const { prefix, suffix } = currentOption?.getLabel() ?? {
    prefix: "",
    suffix: "",
  };

  return (
    <div className="absolute top-3 left-3 z-10">
      <div className="flex items-center gap-2 px-3 py-1 bg-[#2a8af6]/20 text-[#2a8af6] rounded-lg text-sm border border-[#2a8af6]/50 backdrop-blur-sm">
        <button
          onClick={onToggleOpen}
          className="flex items-center gap-1 hover:opacity-80"
          style={{ color: "#2a8af6" }}
        >
          <span>
            {prefix}
            {prefix && " "}
            <strong>{selectedNode}</strong>
            {suffix && " "}
            {suffix}
          </span>
          <svg
            className={`w-3 h-3 transition-transform ${
              isOpen ? "rotate-180" : ""
            }`}
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M19 9l-7 7-7-7"
            />
          </svg>
        </button>
        <button onClick={onClear} className="ml-1 hover:opacity-80">
          ✕
        </button>
      </div>

      {isOpen && (
        <div
          className="mt-1 py-1 rounded-lg border border-[#2a8af6]/50 backdrop-blur-sm shadow-lg"
          style={{ backgroundColor: "var(--ds-background-200)" }}
        >
          {modeOptions.map((option) => {
            const { prefix: optPrefix, suffix: optSuffix } = option.getLabel();
            const isSelected = option.mode === selectionMode;

            return (
              <button
                key={option.mode}
                onClick={() => {
                  onModeChange(option.mode);
                }}
                className="w-full px-3 py-1.5 text-left text-sm hover:bg-[#2a8af6]/20"
                style={{
                  color: isSelected ? "#2a8af6" : "var(--ds-gray-1000)",
                }}
              >
                {optPrefix}
                {optPrefix && " "}
                <strong>{selectedNode}</strong>
                {optSuffix && " "}
                {optSuffix}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function DevtoolsContent() {
  const searchParams = useSearchParams();
  const port = searchParams.get("port");
  const { fitBounds, fitView, getNodes } = useReactFlow();

  const [graphState, setGraphState] = useState<GraphState | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<GraphView>("packages");
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [selectionMode, setSelectionMode] = useState<SelectionMode>("none");
  const [isDropdownOpen, setIsDropdownOpen] = useState(false);
  const [showDisconnected, setShowDisconnected] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [showMinimap, setShowMinimap] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);

  // Store the base (unlayouted) nodes and edges for the current view
  const [baseNodes, setBaseNodes] = useState<Array<Node>>([]);
  const [baseEdges, setBaseEdges] = useState<Array<Edge>>([]);
  const [rawEdges, setRawEdges] = useState<Array<GraphEdge>>([]);

  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);

  // Calculate which nodes/edges should be highlighted based on selection
  const { highlightedNodes, highlightedEdges } = useMemo(() => {
    if (!selectedNode || selectionMode === "none") {
      return { highlightedNodes: null, highlightedEdges: null };
    }

    let visibleNodes: Set<string>;
    if (selectionMode === "direct") {
      visibleNodes = getDirectDependencies(selectedNode, rawEdges);
    } else if (selectionMode === "blocks") {
      visibleNodes = getAffectedNodes(selectedNode, rawEdges);
    } else {
      visibleNodes = getAffectsNodes(selectedNode, rawEdges);
    }

    const visibleEdges = getConnectedEdges(visibleNodes, rawEdges);

    return { highlightedNodes: visibleNodes, highlightedEdges: visibleEdges };
  }, [selectedNode, selectionMode, rawEdges]);

  // Apply highlighting to nodes and edges
  useEffect(() => {
    if (baseNodes.length === 0) return;

    const updatedNodes = baseNodes.map((node) => {
      const isHighlighted = !highlightedNodes || highlightedNodes.has(node.id);
      const isSelected = node.id === selectedNode;

      return {
        ...node,
        selected: isSelected,
        style: {
          ...node.style,
          opacity: isHighlighted ? 1 : 0.2,
        },
      };
    });

    const updatedEdges = baseEdges.map((edge) => {
      const isHighlighted = !highlightedEdges || highlightedEdges.has(edge.id);

      // Use arrow markers for directional modes (blocks/dependsOn)
      // For "blocks" mode: arrows point from selected node outward (shows what it blocks)
      // For "dependsOn" mode: arrows point toward selected node (shows what it depends on)
      const useArrow =
        isHighlighted &&
        (selectionMode === "blocks" || selectionMode === "dependsOn");

      return {
        ...edge,
        markerStart: useArrow ? "edge-arrow" : undefined,
        markerEnd: undefined,
        style: {
          ...edge.style,
          opacity: isHighlighted ? 1 : 0.1,
        },
      };
    });

    setNodes(updatedNodes);
    setEdges(updatedEdges);
  }, [
    baseNodes,
    baseEdges,
    highlightedNodes,
    highlightedEdges,
    selectedNode,
    selectionMode,
    setNodes,
    setEdges,
  ]);

  // Clear selection and reset viewport to show all nodes
  const clearSelection = useCallback(
    (hadFilter: boolean) => {
      setSelectedNode(null);
      setSelectionMode("none");
      // Reset viewport to show all nodes only if we had a filter active
      if (hadFilter) {
        setTimeout(() => {
          fitView();
        }, 50);
      }
    },
    [fitView]
  );

  // Handle node click
  const handleNodeClick: NodeMouseHandler = useCallback(
    (_, node) => {
      // If dropdown is open, just close it
      if (isDropdownOpen) {
        setIsDropdownOpen(false);
        return;
      }

      if (selectedNode === node.id) {
        // Clicking the same node - cycle through modes: direct -> blocks -> dependsOn -> none
        if (selectionMode === "direct") {
          setSelectionMode("blocks");
        } else if (selectionMode === "blocks") {
          setSelectionMode("dependsOn");
        } else if (selectionMode === "dependsOn") {
          clearSelection(true);
        }
      } else {
        // Clicking a different node - start with direct dependencies
        setSelectedNode(node.id);
        setSelectionMode("direct");
      }
    },
    [selectedNode, selectionMode, clearSelection, isDropdownOpen]
  );

  // Handle clicking on the background to clear selection (or just close dropdown)
  const handlePaneClick = useCallback(() => {
    if (isDropdownOpen) {
      setIsDropdownOpen(false);
      return;
    }
    clearSelection(selectionMode !== "none");
  }, [clearSelection, selectionMode, isDropdownOpen]);

  // Get set of node IDs that have at least one edge connection
  const getConnectedNodeIds = useCallback((edges: Array<GraphEdge>) => {
    const connected = new Set<string>();
    for (const edge of edges) {
      connected.add(edge.source);
      connected.add(edge.target);
    }
    return connected;
  }, []);

  // Convert package graph to React Flow elements
  const updatePackageGraphElements = useCallback(
    async (state: GraphState) => {
      // Filter to only nodes that have connections
      const connectedIds = getConnectedNodeIds(state.packageGraph.edges);
      const connectedPackages = state.packageGraph.nodes.filter((pkg) =>
        connectedIds.has(pkg.id)
      );

      const flowNodes: Array<Node<TurboNodeData>> = connectedPackages.map(
        (pkg) => ({
          id: pkg.id,
          type: "turbo",
          data: {
            icon: <Package size={14} />,
            title: pkg.name,
            subtitle: pkg.path || ".",
          },
          position: { x: 0, y: 0 },
        })
      );

      const flowEdges: Array<Edge> = state.packageGraph.edges.map(
        (edge, i) => ({
          id: `e${i}`,
          source: edge.source,
          target: edge.target,
          type: "turbo",
        })
      );

      const { nodes: layoutedNodes, edges: layoutedEdges } =
        await getLayoutedElements(flowNodes, flowEdges);

      setBaseNodes(layoutedNodes);
      setBaseEdges(layoutedEdges);
      setRawEdges(state.packageGraph.edges);
      setNodes(layoutedNodes);
      setEdges(layoutedEdges);
    },
    [setNodes, setEdges, getConnectedNodeIds]
  );

  // Convert task graph to React Flow elements
  const updateTaskGraphElements = useCallback(
    async (state: GraphState) => {
      // Filter to only nodes that have connections
      const connectedIds = getConnectedNodeIds(state.taskGraph.edges);
      const connectedTasks = state.taskGraph.nodes.filter((task) =>
        connectedIds.has(task.id)
      );

      const flowNodes: Array<Node<TurboNodeData>> = connectedTasks.map(
        (task) => ({
          id: task.id,
          type: "turbo",
          data: {
            icon: <FunctionIcon />,
            title: task.task,
            subtitle: task.package,
          },
          position: { x: 0, y: 0 },
        })
      );

      const flowEdges: Array<Edge> = state.taskGraph.edges.map((edge, i) => ({
        id: `e${i}`,
        source: edge.source,
        target: edge.target,
        type: "turbo",
      }));

      const { nodes: layoutedNodes, edges: layoutedEdges } =
        await getLayoutedElements(flowNodes, flowEdges);

      setBaseNodes(layoutedNodes);
      setBaseEdges(layoutedEdges);
      setRawEdges(state.taskGraph.edges);
      setNodes(layoutedNodes);
      setEdges(layoutedEdges);
    },
    [setNodes, setEdges, getConnectedNodeIds]
  );

  // Update flow elements when view or graph state changes
  const updateFlowElements = useCallback(
    async (state: GraphState, currentView: GraphView) => {
      // Clear selection when switching views or updating (don't reset viewport, layout will handle it)
      clearSelection(false);

      if (currentView === "packages") {
        await updatePackageGraphElements(state);
      } else {
        await updateTaskGraphElements(state);
      }
    },
    [updatePackageGraphElements, updateTaskGraphElements, clearSelection]
  );

  // Handle view change
  const handleViewChange = useCallback(
    async (newView: GraphView) => {
      setView(newView);
      if (graphState) {
        await updateFlowElements(graphState, newView);
      }
    },
    [graphState, updateFlowElements]
  );

  // Store latest graphState in a ref for WebSocket handler
  const graphStateRef = useRef<GraphState | null>(null);
  useEffect(() => {
    graphStateRef.current = graphState;
  }, [graphState]);

  // WebSocket connection - only reconnect when port changes
  useEffect(() => {
    if (!port) return;

    const connect = () => {
      const ws = new WebSocket(`ws://localhost:${port}`);
      wsRef.current = ws;

      ws.onopen = () => {
        setIsConnected(true);
        setError(null);
      };

      ws.onmessage = (event) => {
        try {
          const message: ServerMessage = JSON.parse(event.data);
          switch (message.type) {
            case "init":
            case "update":
              if (message.data) {
                setGraphState(message.data);
              }
              break;
            case "ping":
              ws.send(JSON.stringify({ type: "pong" }));
              break;
            case "error":
              setError(message.message ?? "Unknown error");
              break;
          }
        } catch (e) {
          console.error("Failed to parse message:", e);
        }
      };

      ws.onclose = () => {
        setIsConnected(false);
        wsRef.current = null;
      };

      ws.onerror = () => {
        setError("Connection failed");
        setIsConnected(false);
      };
    };

    connect();

    return () => {
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
    };
  }, [port]);

  // Update flow elements when graphState or view changes
  useEffect(() => {
    if (graphState) {
      updateFlowElements(graphState, view);
    }
  }, [graphState, view, updateFlowElements]);

  const nodeLabel = view === "packages" ? "packages" : "tasks";

  // Get nodes that have at least one connection (edge)
  const connectedNodeIds = useMemo(() => {
    const connected = new Set<string>();
    for (const edge of rawEdges) {
      connected.add(edge.source);
      connected.add(edge.target);
    }
    return connected;
  }, [rawEdges]);

  // Get the list of nodes for the sidebar, split into connected and disconnected
  const { connectedNodes, disconnectedNodes } = useMemo(() => {
    if (!graphState) return { connectedNodes: [], disconnectedNodes: [] };

    const allNodes =
      view === "packages"
        ? graphState.packageGraph.nodes.map((pkg) => ({
            id: pkg.id,
            name: pkg.name,
            subtitle: pkg.path,
          }))
        : graphState.taskGraph.nodes.map((task) => ({
            id: task.id,
            name: task.id, // package#task format
            subtitle: task.script,
          }));

    const connected: typeof allNodes = [];
    const disconnected: typeof allNodes = [];

    for (const node of allNodes) {
      if (connectedNodeIds.has(node.id)) {
        connected.push(node);
      } else {
        disconnected.push(node);
      }
    }

    // Sort both lists
    const sortFn = (a: (typeof allNodes)[0], b: (typeof allNodes)[0]) =>
      a.name.localeCompare(b.name) || a.subtitle.localeCompare(b.subtitle);

    return {
      connectedNodes: connected.sort(sortFn),
      disconnectedNodes: disconnected.sort(sortFn),
    };
  }, [graphState, view, connectedNodeIds]);

  // Filter and sort nodes based on search query and highlighting
  const filteredConnectedNodes = useMemo(() => {
    let filtered = connectedNodes;

    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      filtered = connectedNodes.filter(
        (node) =>
          node.name.toLowerCase().includes(query) ||
          node.subtitle.toLowerCase().includes(query)
      );
    }

    // Sort: selected node first, then highlighted nodes, then the rest
    if (selectedNode || highlightedNodes) {
      return [...filtered].sort((a, b) => {
        // Selected node always comes first
        if (a.id === selectedNode) return -1;
        if (b.id === selectedNode) return 1;

        // Then sort by highlighted status
        if (highlightedNodes) {
          const aHighlighted = highlightedNodes.has(a.id);
          const bHighlighted = highlightedNodes.has(b.id);
          if (aHighlighted && !bHighlighted) return -1;
          if (!aHighlighted && bHighlighted) return 1;
        }

        // Within each group, sort alphabetically
        return (
          a.name.localeCompare(b.name) || a.subtitle.localeCompare(b.subtitle)
        );
      });
    }

    return filtered;
  }, [connectedNodes, searchQuery, highlightedNodes, selectedNode]);

  const filteredDisconnectedNodes = useMemo(() => {
    if (!searchQuery.trim()) return disconnectedNodes;
    const query = searchQuery.toLowerCase();
    return disconnectedNodes.filter(
      (node) =>
        node.name.toLowerCase().includes(query) ||
        node.subtitle.toLowerCase().includes(query)
    );
  }, [disconnectedNodes, searchQuery]);

  const nodeCount = connectedNodes.length + disconnectedNodes.length;

  // Focus the viewport on a set of nodes
  const focusOnNodes = useCallback(
    (nodeIds: Set<string>) => {
      const flowNodes = getNodes() as Array<Node<TurboNodeData>>;
      const targetNodes = flowNodes.filter((n) => nodeIds.has(n.id));

      if (targetNodes.length === 0) return;

      // Calculate bounding box of all target nodes
      let minX = Infinity;
      let minY = Infinity;
      let maxX = -Infinity;
      let maxY = -Infinity;

      for (const node of targetNodes) {
        const nodeWidth = calculateNodeWidth(node.data);
        minX = Math.min(minX, node.position.x);
        minY = Math.min(minY, node.position.y);
        maxX = Math.max(maxX, node.position.x + nodeWidth);
        maxY = Math.max(maxY, node.position.y + NODE_HEIGHT);
      }

      // Add padding
      const padding = 50;
      fitBounds(
        {
          x: minX - padding,
          y: minY - padding,
          width: maxX - minX + padding * 2,
          height: maxY - minY + padding * 2,
        },
        { duration: 0 }
      );
    },
    [fitBounds, getNodes]
  );

  // Handle mode change from dropdown
  const handleModeChange = useCallback(
    (mode: ActiveSelectionMode) => {
      if (!selectedNode) return;

      setSelectionMode(mode);
      setIsDropdownOpen(false);

      // Focus on the appropriate nodes for the new mode
      let nodesToFocus: Set<string>;
      if (mode === "direct") {
        nodesToFocus = getDirectDependencies(selectedNode, rawEdges);
      } else if (mode === "blocks") {
        nodesToFocus = getAffectedNodes(selectedNode, rawEdges);
      } else {
        nodesToFocus = getAffectsNodes(selectedNode, rawEdges);
      }
      focusOnNodes(nodesToFocus);
    },
    [selectedNode, rawEdges, focusOnNodes]
  );

  // Handle sidebar node click
  const handleSidebarNodeClick = useCallback(
    (nodeId: string) => {
      if (selectedNode === nodeId) {
        // Clicking the same node - cycle through modes
        if (selectionMode === "direct") {
          setSelectionMode("blocks");
          // Focus on nodes that this blocks (dependents)
          const blocked = getAffectedNodes(nodeId, rawEdges);
          focusOnNodes(blocked);
        } else if (selectionMode === "blocks") {
          setSelectionMode("dependsOn");
          // Focus on nodes that this depends on
          const dependencies = getAffectsNodes(nodeId, rawEdges);
          focusOnNodes(dependencies);
        } else if (selectionMode === "dependsOn") {
          clearSelection(true);
        }
      } else {
        setSelectedNode(nodeId);
        setSelectionMode("direct");
        // Focus on direct dependencies
        const direct = getDirectDependencies(nodeId, rawEdges);
        focusOnNodes(direct);
      }
    },
    [selectedNode, selectionMode, rawEdges, focusOnNodes, clearSelection]
  );

  // No port provided - show instructions
  if (!port) {
    return <SetupInstructions />;
  }

  return (
    <div
      className="fixed left-0 right-0 bottom-0 flex"
      style={{
        top: "var(--nav-height)",
        backgroundColor: "var(--ds-background-100)",
      }}
    >
      {/* Disconnected overlay */}
      {!isConnected && graphState && <DisconnectedOverlay port={port} />}

      {/* Error display */}
      {error && (
        <div className="absolute top-0 left-0 right-0 z-20 px-4 py-2 bg-red-900/30 text-red-400 text-sm">
          Error: {error}
        </div>
      )}

      {/* Sidebar */}
      <aside
        className="w-64 flex flex-col"
        style={{
          borderRight: "1px solid var(--ds-gray-400)",
          backgroundColor: "var(--ds-background-100)",
        }}
      >
        {/* Toggle */}
        <div
          className="px-3 py-3"
          style={{ borderBottom: "1px solid var(--ds-gray-400)" }}
        >
          <GraphViewToggle view={view} onViewChange={handleViewChange} />
        </div>

        {/* Search input */}
        <div
          className="px-3 py-2"
          style={{ borderBottom: "1px solid var(--ds-gray-400)" }}
        >
          <input
            type="text"
            placeholder={`Search ${nodeLabel}...`}
            value={searchQuery}
            onChange={(e) => {
              setSearchQuery(e.target.value);
            }}
            className="w-full px-2 py-1.5 text-sm rounded focus:outline-none placeholder:text-[var(--ds-gray-900)]"
            style={{
              backgroundColor: "var(--ds-gray-200)",
              border: "1px solid var(--ds-gray-400)",
              color: "var(--ds-gray-1000)",
            }}
          />
        </div>

        {/* Node list */}
        <div className="flex-1 overflow-y-auto">
          {/* Connected nodes */}
          {filteredConnectedNodes.map((node) => {
            const isSelected = selectedNode === node.id;
            const isHighlighted =
              !highlightedNodes || highlightedNodes.has(node.id);

            return (
              <button
                key={node.id}
                onClick={() => {
                  handleSidebarNodeClick(node.id);
                }}
                className={`w-full text-left px-3 py-2 transition-colors ${
                  isSelected
                    ? "border-l-2 border-l-[#2a8af6]"
                    : isHighlighted
                    ? ""
                    : "opacity-40"
                }`}
                style={{
                  borderBottom: "1px solid var(--ds-gray-400)",
                  backgroundColor: isSelected
                    ? "var(--ds-gray-200)"
                    : "transparent",
                }}
              >
                <div
                  className="text-sm truncate"
                  style={{ color: "var(--ds-gray-1000)" }}
                >
                  {node.name}
                </div>
                <div
                  className="text-xs truncate"
                  style={{ color: "var(--ds-gray-900)" }}
                >
                  {node.subtitle}
                </div>
              </button>
            );
          })}

          {/* Disconnected nodes section */}
          {filteredDisconnectedNodes.length > 0 && (
            <div style={{ borderTop: "1px solid var(--ds-gray-400)" }}>
              <button
                onClick={() => {
                  setShowDisconnected(!showDisconnected);
                }}
                className="w-full text-left px-3 py-2 text-xs flex items-center justify-between"
                style={{ color: "var(--ds-gray-900)" }}
              >
                <span>
                  {filteredDisconnectedNodes.length} {nodeLabel} with no
                  dependencies
                </span>
                <span>{showDisconnected ? "−" : "+"}</span>
              </button>
              {showDisconnected && (
                <div style={{ backgroundColor: "var(--ds-gray-100)" }}>
                  {filteredDisconnectedNodes.map((node) => (
                    <div
                      key={node.id}
                      className="px-3 py-1.5"
                      style={{ borderBottom: "1px solid var(--ds-gray-400)" }}
                    >
                      <div
                        className="text-xs truncate"
                        style={{ color: "var(--ds-gray-1000)" }}
                      >
                        {node.name}
                      </div>
                      <div
                        className="text-xs truncate"
                        style={{ color: "var(--ds-gray-900)" }}
                      >
                        {node.subtitle}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        {/* Sidebar footer with status */}
        <div
          className="px-3 py-2 space-y-1"
          style={{ borderTop: "1px solid var(--ds-gray-400)" }}
        >
          {graphState && (
            <div className="text-xs" style={{ color: "var(--ds-gray-900)" }}>
              {nodeCount} {nodeLabel}
            </div>
          )}
          <ConnectionStatus isConnected={isConnected} />
        </div>
      </aside>

      {/* Graph */}
      <div className="flex-1 relative">
        {/* Selection indicator overlay */}
        <SelectionIndicator
          selectedNode={selectedNode}
          selectionMode={selectionMode}
          view={view}
          isOpen={isDropdownOpen}
          onToggleOpen={() => {
            setIsDropdownOpen(!isDropdownOpen);
          }}
          onModeChange={handleModeChange}
          onClear={() => {
            clearSelection(true);
            setIsDropdownOpen(false);
          }}
        />
        <ReactFlow
          nodes={nodes}
          edges={edges}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onNodeClick={handleNodeClick}
          onPaneClick={handlePaneClick}
          fitView
          fitViewOptions={{ padding: 0.2 }}
          minZoom={0.1}
          maxZoom={2}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          defaultEdgeOptions={defaultEdgeOptions}
          className="turbo-flow"
        >
          <Controls showInteractive={false}>
            <button
              onClick={() => {
                setShowMinimap(!showMinimap);
              }}
              className="react-flow__controls-button"
              title={showMinimap ? "Hide minimap" : "Show minimap"}
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <rect x="3" y="3" width="18" height="18" rx="2" />
                <rect x="12" y="12" width="7" height="7" rx="1" />
              </svg>
            </button>
          </Controls>
          {showMinimap && <MiniMap pannable zoomable nodeStrokeWidth={3} />}
          <svg>
            <defs>
              <linearGradient
                id="edge-gradient"
                x1="0%"
                y1="0%"
                x2="0%"
                y2="100%"
              >
                <stop offset="0%" stopColor="#ae53ba" />
                <stop offset="100%" stopColor="#2a8af6" />
              </linearGradient>

              {/* Arrow marker for directional highlighting */}
              <marker
                id="edge-arrow"
                viewBox="0 0 10 10"
                refX="6"
                refY="5"
                markerUnits="userSpaceOnUse"
                markerWidth="12"
                markerHeight="12"
                orient="auto-start-reverse"
              >
                <path
                  d="M 0 0 L 10 5 L 0 10 z"
                  fill="#2a8af6"
                  fillOpacity="0.9"
                />
              </marker>
            </defs>
          </svg>
        </ReactFlow>
      </div>
    </div>
  );
}

export function DevtoolsClientComponent() {
  return (
    <Suspense
      fallback={
        <div
          className="flex items-center justify-center min-h-screen"
          style={{ backgroundColor: "var(--ds-background-100)" }}
        >
          <div style={{ color: "var(--ds-gray-900)" }}>Loading...</div>
        </div>
      }
    >
      <ReactFlowProvider>
        <DevtoolsContent />
      </ReactFlowProvider>
    </Suspense>
  );
}
