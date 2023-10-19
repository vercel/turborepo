import type { ReactNode } from "react";

interface Props {
  children?: ReactNode;
}

export function Container({ children }: Props) {
  return <div className="mx-auto max-w-7xl sm:px-6 lg:px-8">{children}</div>;
}
