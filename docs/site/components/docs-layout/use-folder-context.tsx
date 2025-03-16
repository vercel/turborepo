"use client";

import type { ReactNode } from "react";
import { createContext, useContext } from "react";

interface FolderContextProps {
  openFolder: boolean;
  setOpenFolder: (open: boolean) => void;
}

const FolderContext = createContext<FolderContextProps | undefined>(undefined);

export const useFolderContext = (): FolderContextProps => {
  const context = useContext(FolderContext);
  if (!context) {
    throw new Error("useFolderContext must be used within a FolderProvider");
  }
  return context;
};

interface FolderProviderProps {
  children: ReactNode;
  value: FolderContextProps;
}

export const FolderProvider = ({ children, value }: FolderProviderProps) => {
  return (
    <FolderContext.Provider value={value}>{children}</FolderContext.Provider>
  );
};
