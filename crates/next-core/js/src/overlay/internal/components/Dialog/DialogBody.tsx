import * as React from "react";

export type DialogBodyProps = React.PropsWithChildren & {
  className?: string;
};

export function DialogBody({ children, className }: DialogBodyProps) {
  return (
    <div data-nextjs-dialog-body className={className}>
      {children}
    </div>
  );
}
