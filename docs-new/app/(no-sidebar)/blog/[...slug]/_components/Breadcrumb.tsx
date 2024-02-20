"use client";

import { usePathname } from "next/navigation";
import { useBreadcrumb } from "fumadocs-core/breadcrumb";

export const Breadcrumb = (tree: any) => {
  const pathname = usePathname();
  const breadcrumb = useBreadcrumb(pathname, tree);
  console.log(breadcrumb);

  return <p>breadcrumb</p>;
};
