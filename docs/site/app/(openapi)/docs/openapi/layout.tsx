import { Navigation } from "#components/nav/index.tsx";
import { Sidebar } from "#components/docs-layout/sidebar.tsx";
import { baseOptions } from "#app/layout-config.tsx";
import { DocsLayout } from "#components/docs-layout/index.tsx";
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
