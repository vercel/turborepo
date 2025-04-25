import { DocsLayout } from "@/components/docs-layout";
import { repoDocsPages } from "@/app/source";
import { Navigation } from "@/components/nav";
import { Sidebar } from "#/components/docs-layout/sidebar";
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
