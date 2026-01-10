import { notFound } from "next/navigation";
import { enableDevtools } from "../../../flags";
import { DevtoolsClientComponent } from "./devtools-client";
import { createMetadata } from "@/lib/create-metadata";

export const metadata = createMetadata({
  title: "Turborepo Devtools",
  description: "Visualize your Turborepo package and task graphs",
  canonicalPath: "/devtools"
});

export default async function DevtoolsPage() {
  const showDevtools = await enableDevtools();

  if (!showDevtools) {
    return notFound();
  }

  return <DevtoolsClientComponent />;
}
