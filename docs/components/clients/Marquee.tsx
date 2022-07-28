import React from "react";

export const Marquee = React.memo(function Marquee({ children, ...props }) {
  return (
    <div className="overflow-x-hidden">
      <div className="relative" {...props}>
        <div className="inline-block wrapper">{children}</div>
      </div>
    </div>
  );
});
