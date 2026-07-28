import { GeistdocsHomeLayout as PackageHomeLayout } from "@vercel/geistdocs/home-layout";
import type { ComponentProps, ReactNode } from "react";
import { config } from "@/lib/geistdocs/config";

interface HomeLayoutProps {
  children: ReactNode;
  tree: ComponentProps<typeof PackageHomeLayout>["tree"];
}

export const HomeLayout = ({ tree, children }: HomeLayoutProps) => (
  <PackageHomeLayout config={config} tree={tree}>
    {children}
  </PackageHomeLayout>
);
