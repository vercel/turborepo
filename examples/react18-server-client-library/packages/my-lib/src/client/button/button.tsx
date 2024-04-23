"use client";

import { ReactNode } from "react";

interface ButtonProps {
  children: ReactNode;
  className?: string;
  appName: string;
}

/**
 * # Button
 *
 * this is created as client component as it requires client side functionality.
 * Click handlers are not allowed in server components
 *
 * @param children - Button text/contents
 * @param className - CSS className
 * @param appName - appName to display in alert
 * @returns - A react component "Button"
 *
 *
 * Looking for a much more practical example for use of client and
 * server components together in a library? Check out - https://github.com/react18-tools/nextjs-themes
 */
export const Button = ({ children, className, appName }: ButtonProps) => {
  return (
    <button
      className={className}
      onClick={() => alert(`Hello from your ${appName} app!`)}
      data-testid="button"
    >
      {children}
    </button>
  );
};
