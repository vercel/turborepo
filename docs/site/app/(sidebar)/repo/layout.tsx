import type { ReactNode } from "react";
import { DocsLayout } from "#/components/docs-layout";
import { repoDocsPages } from "@/app/source";

export default function Layout({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return <DocsLayout tree={repoDocsPages.pageTree}>{children}</DocsLayout>;
}
