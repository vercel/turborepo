import type { ReactNode } from "react";
import { FaviconHandler } from "../_components/favicon-handler";
import { RedirectsHandler } from "./redirects-handler";

export default function Layout({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return (
    <>
      <RedirectsHandler />
      <FaviconHandler />
      {children}
    </>
  );
}
