"use client";

import { useColorScheme } from "./use-color-scheme";

export type ColorScheme = "light" | "dark";

export function FaviconHandler(): JSX.Element {
  const product = "repo";

  const scheme = useColorScheme();

  return (
    <link
      href={`/images/product-icons/${product}-${scheme}-32x32.png`}
      rel="icon"
      sizes="any"
    />
  );
}
