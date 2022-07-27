import type { ReactNode } from "react";

type Props = {
  children?: ReactNode;
};

export const Container = ({ children }: Props) => {
  return <div className="mx-auto max-w-7xl sm:px-6 lg:px-8">{children}</div>;
};
