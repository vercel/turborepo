import React from "react";

export function Marquee({ children, ...props }) {
  return (
    <div className="overflow-x-hidden">
      <div className="sr-only">
        These are the logos of some but not all of our users.
      </div>
      <div className="relative">
        <div className="inline-block wrapper">{children}</div>
      </div>
    </div>
  );
}
