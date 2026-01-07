import React from "react";

// for when mobile menu is opened
// https://usehooks.com/uselockbodyscroll

export function useLockBodyScroll(enabled: boolean) {
  React.useLayoutEffect(() => {
    if (!enabled) return;

    const originalStyle = window.getComputedStyle(document.body).overflow;
    document.body.style.overflow = "hidden";

    return () => {
      document.body.style.overflow = originalStyle;
    };
  }, [enabled]);
}
