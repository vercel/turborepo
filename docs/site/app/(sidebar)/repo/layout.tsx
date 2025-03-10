import type { ReactNode } from "react";
import { DocsLayout } from "fumadocs-ui/layouts/docs";
import { layoutPropsWithSidebar } from "@/app/_components/inner-layout-props";
import { repoDocsPages } from "@/app/source";

export default function Layout({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return (
    <DocsLayout {...layoutPropsWithSidebar} tree={repoDocsPages.pageTree}>
      {children}
    </DocsLayout>
  );
}
