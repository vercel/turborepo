import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { enableDevtools } from "../../../flags";
import { DevtoolsClientComponent } from "./devtools-client";

export const metadata: Metadata = {
  title: "Turborepo Devtools",
  description: "Visualize your Turborepo package and task graphs"
};

export default async function DevtoolsPage() {
  const showDevtools = await enableDevtools();

  if (!showDevtools) {
    return notFound();
  }

  return <DevtoolsClientComponent />;
}
