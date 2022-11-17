import * as React from "react";

export type ToastProps = React.PropsWithChildren & {
  onClick?: (ev: React.MouseEvent<HTMLDivElement, MouseEvent>) => void;
  className?: string;
};

export function Toast({ onClick, children, className }: ToastProps) {
  return (
    <div data-nextjs-toast onClick={onClick} className={className}>
      <div data-nextjs-toast-wrapper>{children}</div>
    </div>
  );
}
