import * as React from "react";

export type DialogContentProps = React.PropsWithChildren & {
  className?: string;
};

export function DialogContent({ children, className }: DialogContentProps) {
  return (
    <div data-nextjs-dialog-content className={className}>
      {children}
    </div>
  );
}
