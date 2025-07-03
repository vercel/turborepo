import { TreeContextProvider } from "fumadocs-ui/provider";
import type { PageTree } from "fumadocs-core/server";
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarInset,
} from "#components/ui/sidebar.tsx";
import { LayoutBody, TableOfContents, SidebarItems } from "./docs.client";
import { SidebarViewport } from "./sidebar";
import { MobileMenu } from "./mobile-menu";
import { MobileMenuProvider } from "./use-mobile-menu-context";

interface DocsLayoutProps {
  tree: PageTree.Root;
  children: React.ReactNode;
  path?: Array<string>;
  isOpenApiSpec?: boolean;
}

export const DocsLayout = ({
  tree,
  children,
  isOpenApiSpec,
}: DocsLayoutProps) => {
  return (
    <TreeContextProvider tree={tree}>
      <LayoutBody isOpenApiSpec={isOpenApiSpec}>
        <Sidebar className="sticky left-auto top-[calc(var(--nav-height)+32px)] h-[calc(100svh-var(--nav-height)-64px)] justify-self-end border-none">
          <SidebarViewport>
            <SidebarContent>
              <SidebarGroup className="px-6">
                <SidebarItems />
              </SidebarGroup>
            </SidebarContent>
          </SidebarViewport>
        </Sidebar>
        <SidebarInset>
          <div className="flex w-full flex-row gap-x-6 [&_article]:mt-[var(--mobile-menu-height)] md:[&_article]:mt-0 md:[&_article]:px-0">
            <div className="grid w-full max-w-3xl grid-cols-1 gap-10 px-0 md:pr-4 xl:mx-auto xl:px-0">
              <MobileMenuProvider>
                <MobileMenu />
              </MobileMenuProvider>
              {children}
            </div>
            {isOpenApiSpec ? null : (
              <aside
                id="nd-toc"
                className="sticky top-[calc(var(--nav-height)+32px)] hidden h-fit shrink-0 flex-col gap-2.5 overflow-x-hidden p-2 md:w-[256px] xl:flex 2xl:w-72"
              >
                <TableOfContents />
              </aside>
            )}
          </div>
        </SidebarInset>
      </LayoutBody>
    </TreeContextProvider>
  );
};
