import * as React from "react";

import { lock, unlock } from "./body-locker";
import allyTrap from "./maintain--tab-focus";

export type OverlayProps = React.PropsWithChildren & {
  className?: string;
  fixed?: boolean;
};

export function Overlay({ className, children, fixed }: OverlayProps) {
  React.useEffect(() => {
    lock();
    return () => {
      unlock();
    };
  }, []);

  const [overlay, setOverlay] = React.useState<HTMLDivElement | null>(null);
  const onOverlay = React.useCallback((el: HTMLDivElement) => {
    setOverlay(el);
  }, []);

  React.useEffect(() => {
    if (overlay == null) {
      return;
    }

    const handle2 = allyTrap({ context: overlay });
    return () => {
      handle2.disengage();
    };
  }, [overlay]);

  return (
    <div data-nextjs-dialog-overlay className={className} ref={onOverlay}>
      <div
        data-nextjs-dialog-backdrop
        data-nextjs-dialog-backdrop-fixed={fixed ? true : undefined}
      />
      {children}
    </div>
  );
}
