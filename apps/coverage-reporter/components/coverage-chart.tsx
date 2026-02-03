"use client";

import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Legend
} from "recharts";

interface DataPoint {
  timestamp: string;
  lines: number;
  functions: number;
  branches: number;
}

interface CoverageChartProps {
  data: DataPoint[];
}

export function CoverageChart({ data }: CoverageChartProps) {
  // Format timestamp for display
  const formattedData = data.map((d) => ({
    ...d,
    date: new Date(d.timestamp).toLocaleDateString("en-US", {
      month: "short",
      day: "numeric"
    })
  }));

  return (
    <ResponsiveContainer width="100%" height={300}>
      <LineChart data={formattedData}>
        <CartesianGrid strokeDasharray="3 3" stroke="#333" />
        <XAxis dataKey="date" stroke="#888" fontSize={12} />
        <YAxis stroke="#888" fontSize={12} domain={[0, 100]} unit="%" />
        <Tooltip
          contentStyle={{
            background: "#141414",
            border: "1px solid #333",
            borderRadius: "4px"
          }}
          labelStyle={{ color: "#888" }}
        />
        <Legend />
        <Line
          type="monotone"
          dataKey="lines"
          stroke="#3291ff"
          strokeWidth={2}
          dot={false}
          name="Lines"
        />
        <Line
          type="monotone"
          dataKey="functions"
          stroke="#50e3c2"
          strokeWidth={2}
          dot={false}
          name="Functions"
        />
        <Line
          type="monotone"
          dataKey="branches"
          stroke="#f5a623"
          strokeWidth={2}
          dot={false}
          name="Branches"
        />
      </LineChart>
    </ResponsiveContainer>
  );
}
