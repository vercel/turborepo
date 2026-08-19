"use client";

import { type ReactNode, useCallback } from "react";

interface ButtonProps {
  appName: string;
  children: ReactNode;
  className?: string;
}

export const Button = ({ children, className, appName }: ButtonProps) => {
  const handleClick = useCallback(() => {
    console.log(`Hello from your ${appName} app!`);
  }, [appName]);

  return (
    <button className={className} onClick={handleClick} type="button">
      {children}
    </button>
  );
};
