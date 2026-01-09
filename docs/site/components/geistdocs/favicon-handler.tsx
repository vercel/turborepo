"use client";

import { useEffect, useState } from "react";

type ColorScheme = "light" | "dark";

export const FaviconHandler = () => {
  const [colorScheme, setColorScheme] = useState<ColorScheme>("light");
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");

    // Set initial value
    setColorScheme(mediaQuery.matches ? "dark" : "light");
    setMounted(true);

    const handleChange = (event: MediaQueryListEvent): void => {
      setColorScheme(event.matches ? "dark" : "light");
    };

    mediaQuery.addEventListener("change", handleChange);

    return () => {
      mediaQuery.removeEventListener("change", handleChange);
    };
  }, []);

  // Avoid hydration mismatch by not rendering until mounted
  if (!mounted) {
    return null;
  }

  return (
    <link
      href={`/images/product-icons/repo-${colorScheme}-32x32.png`}
      rel="icon"
      sizes="any"
    />
  );
};
