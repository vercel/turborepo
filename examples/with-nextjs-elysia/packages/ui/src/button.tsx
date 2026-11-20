"use client";

import type { ReactNode } from "react";
import { useCallback } from "react";

interface ButtonProps {
  appName: string;
  children: ReactNode;
  className?: string;
}

export const Button = ({ children, className, appName }: ButtonProps) => {
  const handleClick = useCallback(() => {
    // biome-ignore lint/suspicious/noAlert: Demo interaction for the starter component
    alert(`Hello from your ${appName} app!`);
  }, [appName]);

  return (
    <button className={className} onClick={handleClick} type="button">
      {children}
    </button>
  );
};
