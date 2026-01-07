import type { ReactElement, ReactNode } from "react";

// Simplified version that always shows the content
// In the old site, this checked if the turbo version met a requirement
export function InVersion({
  children
}: {
  version: string;
  children: ReactNode;
}): ReactElement {
  return <>{children}</>;
}
