import type { Metadata } from "next";
import { DevtoolsClientComponent } from "./devtools-client";
import { createMetadata } from "@/lib/create-metadata";
import { getLocalizedPath } from "@/lib/geistdocs/public-path";

export const generateMetadata = async ({
  params
}: PageProps<"/[lang]/devtools">): Promise<Metadata> => {
  const { lang } = await params;

  return createMetadata({
    title: "Turborepo Devtools",
    description: "Visualize your Turborepo package and task graphs",
    canonicalPath: getLocalizedPath(lang, "/devtools")
  });
};

export default function DevtoolsPage() {
  return <DevtoolsClientComponent />;
}
