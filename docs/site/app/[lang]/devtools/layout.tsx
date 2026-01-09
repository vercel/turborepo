import type { ReactNode } from "react";

export default function DevtoolsLayout({ children }: { children: ReactNode }) {
  // No extra wrapper - devtools uses fixed positioning
  return <>{children}</>;
}
