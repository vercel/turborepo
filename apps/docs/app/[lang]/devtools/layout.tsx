import type { ReactNode } from "react";

export default function DevtoolsLayout({ children }: { children: ReactNode }) {
  return <main className="contents">{children}</main>;
}
