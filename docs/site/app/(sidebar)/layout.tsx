import { DocsLayout } from "#components/docs-layout/index.tsx";
import { repoDocsPages } from "#app/source.ts";
import { Navigation } from "#components/nav/index.tsx";
import { Sidebar } from "#components/docs-layout/sidebar.tsx";
import { baseOptions } from "../layout-config";
import { RedirectsHandler } from "./redirects-handler";

export default function Layout({ children }: { children: React.ReactNode }) {
  return (
    <>
      <Navigation />
      <Sidebar>
        <DocsLayout tree={repoDocsPages.pageTree} {...baseOptions}>
          {children}
        </DocsLayout>
      </Sidebar>
      <RedirectsHandler />
    </>
  );
}
