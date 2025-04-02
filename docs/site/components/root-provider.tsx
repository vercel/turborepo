"use client";

import { RootProvider as FumaRootProvider } from "fumadocs-ui/provider";
import type { ReactNode } from "react";
import { SearchDialog } from "@/components/search-dialog";
import { LocalStorageProvider } from "./local-storage-hook";
import { TopLevelMobileMenuProvider } from "./docs-layout/use-mobile-menu-context";
import { AnalyticsScripts } from "./analytics/analytics-scripts";

export function RootProvider({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return (
    <AnalyticsScripts>
      <TopLevelMobileMenuProvider>
        <LocalStorageProvider>
          <FumaRootProvider search={{ SearchDialog }}>
            {children}
          </FumaRootProvider>
        </LocalStorageProvider>
      </TopLevelMobileMenuProvider>
    </AnalyticsScripts>
  );
}
