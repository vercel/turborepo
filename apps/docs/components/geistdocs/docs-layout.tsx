import { GeistdocsDocsLayout as PackageDocsLayout } from "@vercel/geistdocs/layout";
import type { ComponentProps, ReactNode } from "react";
import { VersionWarning } from "@/components/version-warning";
import { config } from "@/lib/geistdocs/config";

interface DocsLayoutProps {
  children: ReactNode;
  tree: ComponentProps<typeof PackageDocsLayout>["tree"];
}

export const DocsLayout = ({ tree, children }: DocsLayoutProps) => (
  <PackageDocsLayout
    config={config}
    containerProps={{
      className: "mx-auto max-w-[1448px]"
    }}
    sidebarTop={<VersionWarning />}
    tree={tree}
  >
    {children}
  </PackageDocsLayout>
);
