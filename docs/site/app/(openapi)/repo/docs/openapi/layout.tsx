import type { ReactNode } from "react";
import { DocsLayout } from "fumadocs-ui/layouts/docs";
import { openapiPages } from "@/app/(openapi)/repo/source";

export default function Layout({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return (
    <DocsLayout
      sidebar={{ defaultOpenLevel: 0, collapsible: false }}
      tree={openapiPages.pageTree}
    >
      {children}
    </DocsLayout>
  );
}
