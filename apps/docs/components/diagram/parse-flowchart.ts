/**
 * Parses a subset of the Mermaid flowchart DSL into a structured graph.
 *
 * Supported syntax:
 *   graph LR | graph TD | graph TB | flowchart LR | flowchart TD
 *   A[Label] --> B[Label]       (arrow edge)
 *   A --> |edge label| B        (labeled edge)
 *   A --- B                     (plain link)
 *   A -.- B                     (dotted link)
 *   A ==> B                     (thick arrow)
 *   A((Label))                  (circle node)
 *   A{Label}                    (diamond node)
 *   A([Label])                  (stadium / pill)
 *   A[[Label]]                  (subroutine)
 *   A>Label]                    (asymmetric)
 *   subgraph id [Title] ... end
 */

export type FlowDirection = "LR" | "TD" | "TB" | "RL" | "BT";

export interface FlowNode {
  id: string;
  label: string;
  shape:
    | "rect"
    | "round"
    | "circle"
    | "diamond"
    | "stadium"
    | "subroutine"
    | "asymmetric";
  subgraph?: string;
}

export interface FlowEdge {
  source: string;
  target: string;
  label?: string;
  style: "arrow" | "plain" | "dotted" | "thick";
}

export interface FlowSubgraph {
  id: string;
  title: string;
  nodeIds: string[];
}

export interface FlowchartGraph {
  direction: FlowDirection;
  nodes: FlowNode[];
  edges: FlowEdge[];
  subgraphs: FlowSubgraph[];
}

function parseNodeDeclaration(raw: string): {
  id: string;
  label: string;
  shape: FlowNode["shape"];
} {
  // Circle: A((label))
  let match = raw.match(/^([a-zA-Z_][\w-]*)\(\((.+?)\)\)$/);
  if (match) return { id: match[1], label: match[2], shape: "circle" };

  // Stadium: A([label])
  match = raw.match(/^([a-zA-Z_][\w-]*)\(\[(.+?)\]\)$/);
  if (match) return { id: match[1], label: match[2], shape: "stadium" };

  // Subroutine: A[[label]]
  match = raw.match(/^([a-zA-Z_][\w-]*)\[\[(.+?)\]\]$/);
  if (match) return { id: match[1], label: match[2], shape: "subroutine" };

  // Diamond: A{label}
  match = raw.match(/^([a-zA-Z_][\w-]*)\{(.+?)\}$/);
  if (match) return { id: match[1], label: match[2], shape: "diamond" };

  // Asymmetric: A>label]
  match = raw.match(/^([a-zA-Z_][\w-]*)>(.+?)\]$/);
  if (match) return { id: match[1], label: match[2], shape: "asymmetric" };

  // Round rect: A(label)
  match = raw.match(/^([a-zA-Z_][\w-]*)\((.+?)\)$/);
  if (match) return { id: match[1], label: match[2], shape: "round" };

  // Rect: A[label]
  match = raw.match(/^([a-zA-Z_][\w-]*)\[(.+?)\]$/);
  if (match) return { id: match[1], label: match[2], shape: "rect" };

  // Bare ID
  match = raw.match(/^([a-zA-Z_][\w-]*)$/);
  if (match) return { id: match[1], label: match[1], shape: "rect" };

  return { id: raw, label: raw, shape: "rect" };
}

// Strips surrounding quotes from a label if present
function stripQuotes(s: string): string {
  if (
    (s.startsWith('"') && s.endsWith('"')) ||
    (s.startsWith("'") && s.endsWith("'"))
  ) {
    return s.slice(1, -1);
  }
  return s;
}

/**
 * Tokenizes an edge statement into its component parts.
 *
 * We scan left-to-right looking for edge operators (-->, ---, -.- , ==>)
 * and optional labels (|label|). Node references can contain brackets so
 * we can't naively split on whitespace.
 */
function parseEdgeStatement(
  line: string,
  nodeMap: Map<string, FlowNode>
): FlowEdge[] {
  const edges: FlowEdge[] = [];
  let remaining = line.trim();

  // Split the chain: A --> B --> C
  // We need to find all edges in a chain
  const parts: string[] = [];
  const operators: { style: FlowEdge["style"]; label?: string }[] = [];

  while (remaining.length > 0) {
    // Try to find the next edge operator
    // Look for: ==> , --> , --- , -.- , -.->
    const opMatch = remaining.match(
      /^(.+?)\s+(==+>|--+>|--+-|--+|(?:-\.+-?>?))\s*(?:\|([^|]*)\|)?\s*/
    );

    if (!opMatch) {
      // No more operators — rest is a node
      parts.push(remaining.trim());
      break;
    }

    parts.push(opMatch[1].trim());

    const op = opMatch[2];
    const label = opMatch[3] ? stripQuotes(opMatch[3].trim()) : undefined;

    let style: FlowEdge["style"] = "arrow";
    if (op.startsWith("==")) {
      style = "thick";
    } else if (op.startsWith("-.")) {
      style = op.endsWith(">") ? "dotted" : "dotted";
    } else if (op.endsWith(">")) {
      style = "arrow";
    } else {
      style = "plain";
    }

    operators.push({ style, label });
    remaining = remaining.slice(opMatch[0].length);
  }

  // Register all nodes and build edges
  const resolvedIds: string[] = [];
  for (const part of parts) {
    const parsed = parseNodeDeclaration(part);
    if (!nodeMap.has(parsed.id)) {
      nodeMap.set(parsed.id, {
        id: parsed.id,
        label: parsed.label,
        shape: parsed.shape
      });
    } else if (parsed.label !== parsed.id) {
      // Update label if a richer declaration is found
      const existing = nodeMap.get(parsed.id)!;
      existing.label = parsed.label;
      existing.shape = parsed.shape;
    }
    resolvedIds.push(parsed.id);
  }

  for (let i = 0; i < operators.length; i++) {
    if (resolvedIds[i] && resolvedIds[i + 1]) {
      edges.push({
        source: resolvedIds[i],
        target: resolvedIds[i + 1],
        label: operators[i].label,
        style: operators[i].style
      });
    }
  }

  return edges;
}

export function parseFlowchart(input: string): FlowchartGraph {
  const lines = input
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);

  let direction: FlowDirection = "TD";
  const nodeMap = new Map<string, FlowNode>();
  const edges: FlowEdge[] = [];
  const subgraphs: FlowSubgraph[] = [];
  const subgraphStack: FlowSubgraph[] = [];

  let startIndex = 0;

  // Parse header
  if (lines.length > 0) {
    const header = lines[0];
    const headerMatch = header.match(
      /^(?:graph|flowchart)\s+(LR|TD|TB|RL|BT)$/i
    );
    if (headerMatch) {
      direction = headerMatch[1].toUpperCase() as FlowDirection;
      startIndex = 1;
    } else if (header.match(/^(?:graph|flowchart)$/i)) {
      startIndex = 1;
    }
  }

  for (let i = startIndex; i < lines.length; i++) {
    const line = lines[i];

    // Skip comments
    if (line.startsWith("%%")) continue;

    // Subgraph start: subgraph id [Title]
    const subgraphMatch = line.match(/^subgraph\s+(\S+)(?:\s*\[(.+?)\])?\s*$/i);
    if (subgraphMatch) {
      const sg: FlowSubgraph = {
        id: subgraphMatch[1],
        title: subgraphMatch[2] ?? subgraphMatch[1],
        nodeIds: []
      };
      subgraphs.push(sg);
      subgraphStack.push(sg);
      continue;
    }

    // Subgraph end
    if (line.toLowerCase() === "end") {
      subgraphStack.pop();
      continue;
    }

    // Edge statement (contains an operator)
    if (line.match(/-->|---|==>|-\.-|-\.->|--[^-]/)) {
      const newEdges = parseEdgeStatement(line, nodeMap);
      edges.push(...newEdges);

      // If inside a subgraph, track node membership
      if (subgraphStack.length > 0) {
        const currentSg = subgraphStack[subgraphStack.length - 1];
        for (const edge of newEdges) {
          if (!currentSg.nodeIds.includes(edge.source)) {
            currentSg.nodeIds.push(edge.source);
          }
          if (!currentSg.nodeIds.includes(edge.target)) {
            currentSg.nodeIds.push(edge.target);
          }
        }
      }
      continue;
    }

    // Standalone node declaration
    const parsed = parseNodeDeclaration(line);
    if (parsed.id) {
      if (!nodeMap.has(parsed.id)) {
        nodeMap.set(parsed.id, {
          id: parsed.id,
          label: parsed.label,
          shape: parsed.shape
        });
      }

      if (subgraphStack.length > 0) {
        const currentSg = subgraphStack[subgraphStack.length - 1];
        if (!currentSg.nodeIds.includes(parsed.id)) {
          currentSg.nodeIds.push(parsed.id);
        }
      }
    }
  }

  // Assign subgraph membership to nodes
  for (const sg of subgraphs) {
    for (const nodeId of sg.nodeIds) {
      const node = nodeMap.get(nodeId);
      if (node) {
        node.subgraph = sg.id;
      }
    }
  }

  return {
    direction,
    nodes: Array.from(nodeMap.values()),
    edges,
    subgraphs
  };
}
