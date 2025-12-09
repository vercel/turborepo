"use client";

import {
  createContext,
  useContext,
  useState,
  useEffect,
  type ReactNode,
} from "react";
import type { TOCItemType } from "fumadocs-core/server";

interface TOCContextValue {
  toc: Array<TOCItemType> | null;
  setTOC: (toc: Array<TOCItemType> | null) => void;
}

const TOCContext = createContext<TOCContextValue | null>(null);

export function TOCProvider({ children }: { children: ReactNode }) {
  const [toc, setTOC] = useState<Array<TOCItemType> | null>(null);

  return (
    <TOCContext.Provider value={{ toc, setTOC }}>
      {children}
    </TOCContext.Provider>
  );
}

export function useTOCContext() {
  const context = useContext(TOCContext);
  if (!context) {
    throw new Error("useTOCContext must be used within a TOCProvider");
  }
  return context;
}

export function useTOC() {
  const context = useContext(TOCContext);
  return context?.toc ?? null;
}

/**
 * Client component that sets the TOC in context.
 * Should be rendered by the nested layout that has access to the page data.
 */
export function TOCSetter({ toc }: { toc: Array<TOCItemType> }) {
  const context = useContext(TOCContext);

  useEffect(() => {
    context?.setTOC(toc);
    return () => {
      context?.setTOC(null);
    };
  }, [toc, context]);

  return null;
}
