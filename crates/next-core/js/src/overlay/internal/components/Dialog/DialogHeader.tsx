import * as React from "react";

export type DialogHeaderProps = React.PropsWithChildren & {
  className?: string;
};

export function DialogHeader({ children, className }: DialogHeaderProps) {
  return (
    <div data-nextjs-dialog-header className={className}>
      {children}
    </div>
  );
}
