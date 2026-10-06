import type { ButtonHTMLAttributes, ReactNode, ReactElement } from "react";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  children: ReactNode;
}

export function Button({ children, ...other }: ButtonProps): ReactElement {
  return (
    <button type="button" {...other}>
      {children}
    </button>
  );
}

Button.displayName = "Button";
