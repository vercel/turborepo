import React from "react";

export function Filters() {
  return (
    <>
      <svg height={0} width={0}>
        <defs>
          <filter id="high-threshold">
            <feColorMatrix type="saturate" values="0" />
            <feComponentTransfer>
              <feFuncR tableValues="0" type="discrete" />
              <feFuncG tableValues="0" type="discrete" />
              <feFuncB tableValues="0" type="discrete" />
            </feComponentTransfer>
          </filter>
        </defs>
      </svg>
      <svg height={0} width={0}>
        <defs>
          <filter id="medium-threshold">
            <feColorMatrix type="saturate" values="0" />
            <feComponentTransfer>
              <feFuncR tableValues="0 1" type="discrete" />
              <feFuncG tableValues="0 1" type="discrete" />
              <feFuncB tableValues="0 1" type="discrete" />
            </feComponentTransfer>
          </filter>
        </defs>
      </svg>
      <svg height={0} width={0}>
        <defs>
          <filter id="low-threshold">
            <feColorMatrix type="saturate" values="0" />
            <feComponentTransfer>
              <feFuncR tableValues="0 0 0 0 1" type="discrete" />
              <feFuncG tableValues="0 0 0 0 1" type="discrete" />
              <feFuncB tableValues="0 0 0 0 1" type="discrete" />
            </feComponentTransfer>
          </filter>
        </defs>
      </svg>
    </>
  );
}
