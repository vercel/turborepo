"use client";

import type { ReactNode } from "react";
import { createContext, useContext, useState } from "react";

interface MobileMenuContextProps {
  openMobileMenu: boolean;
  setOpenMobileMenu: (open: boolean) => void;
}

const MobileMenuContext = createContext<MobileMenuContextProps | undefined>(
  undefined
);

export const useMobileMenuContext = (): MobileMenuContextProps => {
  const context = useContext(MobileMenuContext);
  if (!context) {
    throw new Error(
      "useMobileMenuContext must be used within a MobileMenuProvider"
    );
  }
  return context;
};

interface MobileMenuProviderProps {
  children: ReactNode;
}

export const MobileMenuProvider = ({ children }: MobileMenuProviderProps) => {
  const [openMobileMenu, setOpenMobileMenu] = useState(false);

  return (
    <MobileMenuContext.Provider value={{ openMobileMenu, setOpenMobileMenu }}>
      {children}
    </MobileMenuContext.Provider>
  );
};

interface TopLevelMobileMenuContextProps {
  openMobileMenu: boolean;
  setOpenMobileMenu: (open: boolean) => void;
}

const TopLevelMobileMenuContext = createContext<
  TopLevelMobileMenuContextProps | undefined
>(undefined);

export const useTopLevelMobileMenuContext = (): MobileMenuContextProps => {
  const context = useContext(TopLevelMobileMenuContext);
  if (!context) {
    throw new Error(
      "useMobileMenuContext must be used within a MobileMenuProvider"
    );
  }
  return context;
};

export const TopLevelMobileMenuProvider = ({
  children,
}: MobileMenuProviderProps) => {
  const [openMobileMenu, setOpenMobileMenu] = useState(false);

  return (
    <TopLevelMobileMenuContext.Provider
      value={{ openMobileMenu, setOpenMobileMenu }}
    >
      {children}
    </TopLevelMobileMenuContext.Provider>
  );
};
