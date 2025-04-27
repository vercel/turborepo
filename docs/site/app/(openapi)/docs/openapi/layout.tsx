import { DocsLayout } from "@/components/docs-layout";
import { Navigation } from "@/components/nav";
import { Sidebar } from "#components/docs-layout/sidebar.tsx";
import { baseOptions } from "#app/layout-config.tsx";
import { openapiPages } from "./source";

export default function Layout({ children }: { children: React.ReactNode }) {
  return (
    <>
      <Navigation />
      <Sidebar>
        <DocsLayout isOpenApiSpec tree={openapiPages.pageTree} {...baseOptions}>
          {children}
        </DocsLayout>
      </Sidebar>
    </>
  );
}
