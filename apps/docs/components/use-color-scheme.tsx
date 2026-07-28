"use client";

import { useState, useEffect } from "react";

export type ColorScheme = "light" | "dark";

export function useColorScheme(): ColorScheme {
  const [colorScheme, setColorScheme] = useState<ColorScheme>(() => {
    // Check the initial color scheme
    /* eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- Window check is necessary for SSR */
    if (typeof window !== "undefined" && window.matchMedia) {
      return window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light";
    }
    return "light"; // Default to light if matchMedia is not available
  });

  useEffect(() => {
    if (typeof window === "undefined") return;

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");

    // Set initial value on mount (SSR defaults to "light", so we need to sync)
    setColorScheme(mediaQuery.matches ? "dark" : "light");

    const handleChange = (event: MediaQueryListEvent): void => {
      setColorScheme(event.matches ? "dark" : "light");
    };

    // Add event listener
    mediaQuery.addEventListener("change", handleChange);

    // Clean up
    return () => {
      mediaQuery.removeEventListener("change", handleChange);
    };
  }, []);

  return colorScheme;
}
