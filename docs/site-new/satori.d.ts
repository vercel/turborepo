// Type declarations for Satori's tw prop used in next/og ImageResponse
// See: https://github.com/vercel/satori#tailwind-css

import type { CSSProperties } from "react";

declare module "react" {
  interface HTMLAttributes<T> {
    tw?: string;
  }

  interface SVGAttributes<T> {
    tw?: string;
  }
}
