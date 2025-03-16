"use client";

import { RootProvider as FumaRootProvider } from "fumadocs-ui/provider";
import type { ReactNode } from "react";
import { SearchDialog } from "@/components/search-dialog";
import { LocalStorageProvider } from "./local-storage-hook";
import { SidebarProvider } from "./ui/sidebar";

export function RootProvider({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return (
    <LocalStorageProvider>
      <SidebarProvider>
        <FumaRootProvider search={{ SearchDialog }}>
          {children}
        </FumaRootProvider>
      </SidebarProvider>
    </LocalStorageProvider>
  );
}
