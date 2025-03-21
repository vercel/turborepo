"use client";

import { RootProvider as FumaRootProvider } from "fumadocs-ui/provider";
import type { ReactNode } from "react";
import { SearchDialog } from "@/components/search-dialog";
import { LocalStorageProvider } from "./local-storage-hook";
import { TopLevelMobileMenuProvider } from "./docs-layout/use-mobile-menu-context";

export function RootProvider({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return (
    <TopLevelMobileMenuProvider>
      <LocalStorageProvider>
        <FumaRootProvider search={{ SearchDialog }}>
          {children}
        </FumaRootProvider>
      </LocalStorageProvider>
    </TopLevelMobileMenuProvider>
  );
}
