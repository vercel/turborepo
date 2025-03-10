"use client";

import { usePathname } from "next/navigation";
import { useColorScheme } from "./use-color-scheme";

export type ColorScheme = "light" | "dark";

export function FaviconHandler(): JSX.Element {
  const pathname = usePathname();
  const productFromSlug = pathname.split("/")[1]?.length
    ? pathname.split("/")[1]
    : "repo";

  const product =
    productFromSlug === "repo" || productFromSlug === "pack"
      ? productFromSlug
      : "repo";

  const scheme = useColorScheme();

  return (
    <link
      href={`/images/product-icons/${product}-${scheme}-32x32.png`}
      rel="icon"
      sizes="any"
    />
  );
}
