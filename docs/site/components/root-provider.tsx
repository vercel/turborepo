"use client";

import { RootProvider as FumaRootProvider } from "fumadocs-ui/provider";
import type { ReactNode } from "react";
import { SearchDialog } from "#components/search-dialog.tsx";
import { TopLevelMobileMenuProvider } from "./docs-layout/use-mobile-menu-context";

export function RootProvider({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return (
    <TopLevelMobileMenuProvider>
      <FumaRootProvider search={{ SearchDialog }}>{children}</FumaRootProvider>
    </TopLevelMobileMenuProvider>
  );
}
