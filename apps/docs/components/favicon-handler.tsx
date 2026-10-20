"use client";

import type { JSX } from "react";
import { useColorScheme } from "./use-color-scheme";
import ProductIconDark from "../public/images/product-icons/repo-dark-32x32.png";
import ProductIconLight from "../public/images/product-icons/repo-light-32x32.png";

export function FaviconHandler(): JSX.Element {
  const scheme = useColorScheme();

  const whichIcon = scheme === "dark" ? ProductIconDark : ProductIconLight;

  return <link href={whichIcon.src} rel="icon" sizes="any" />;
}
