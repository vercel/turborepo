import type { Metadata } from "next";
import { DevtoolsClientComponent } from "./devtools-client";

export const metadata: Metadata = {
  title: "Turbo Devtools",
  description: "Visualize your Turborepo package and task graphs",
};

export default function ToolsPage() {
  return <DevtoolsClientComponent />;
}
