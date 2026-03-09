"use client";

import { useMemo } from "react";
import { parseFlowchart } from "./parse-flowchart";
import { parseSequenceDiagram } from "./parse-sequence";
import { FlowDiagram } from "./flow-diagram";
import { SequenceDiagram } from "./sequence-diagram";

function detectDiagramType(
  chart: string
): "flowchart" | "sequence" | "unknown" {
  const firstLine = chart.trim().split("\n")[0]?.trim().toLowerCase() ?? "";

  if (
    firstLine === "sequencediagram" ||
    firstLine.startsWith("sequencediagram")
  ) {
    return "sequence";
  }

  if (
    firstLine.startsWith("graph ") ||
    firstLine.startsWith("flowchart ") ||
    firstLine === "graph" ||
    firstLine === "flowchart"
  ) {
    return "flowchart";
  }

  return "unknown";
}

interface MermaidProps {
  chart: string;
}

export function Mermaid({ chart }: MermaidProps) {
  const normalizedChart = chart.replaceAll("\\n", "\n");
  const type = detectDiagramType(normalizedChart);

  const content = useMemo(() => {
    if (type === "flowchart") {
      const graph = parseFlowchart(normalizedChart);
      return <FlowDiagram graph={graph} />;
    }

    if (type === "sequence") {
      const diagram = parseSequenceDiagram(normalizedChart);
      return <SequenceDiagram diagram={diagram} />;
    }

    return (
      <pre className="my-6 rounded-lg border p-4 text-sm overflow-x-auto">
        <code>{normalizedChart}</code>
      </pre>
    );
  }, [normalizedChart, type]);

  return content;
}
