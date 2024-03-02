import type { ReactNode } from "react";

interface ContainerProps {
  children?: ReactNode;
}

export function Container({ children }: ContainerProps) {
  return <div className="mx-auto max-w-7xl sm:px-6 lg:px-8">{children}</div>;
}
