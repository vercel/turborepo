import type { ReactNode } from "react";
import { HomeLayout } from "fumadocs-ui/layouts/home";
import { FaviconHandler } from "../_components/favicon-handler";
import { baseOptions } from "../layout-config";

export default function Layout({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return (
    <>
      <FaviconHandler />
      {/* @ts-expect-error - Type incompatibility between HomeLayout props and baseOptions */}
      <HomeLayout className="p-0" {...baseOptions}>
        {children}
      </HomeLayout>
    </>
  );
}
