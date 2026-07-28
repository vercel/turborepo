import { DevtoolsClientComponent } from "./devtools-client";
import { createMetadata } from "@/lib/create-metadata";

export const metadata = createMetadata({
  title: "Turborepo Devtools",
  description: "Visualize your Turborepo package and task graphs",
  canonicalPath: "/devtools"
});

export default function DevtoolsPage() {
  return <DevtoolsClientComponent />;
}
