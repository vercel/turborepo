"use client";

import { usePathname } from "next/navigation";
import { useEffect } from "react";

// Responsible for highlighting the row in the table
// of the variable found in the hash
export function SystemEnvironmentVariablesHashHighlighter(): JSX.Element {
  const path = usePathname();

  useEffect(() => {
    const hash = window.location.hash.substring(1);
    if (path === "/docs/reference/system-environment-variables" && hash) {
      const element = document.getElementById(hash);
      if (!element) return;
      element.classList.add("focus");
    }
  }, [path]);

  return <></>;
}
