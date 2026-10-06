import type { ReactNode } from "react";

interface WorkLayoutProps {
  readonly children: ReactNode;
}

export default function WorkLayout({ children }: WorkLayoutProps) {
  return children;
}
