import cn from "classnames";
import { z } from "zod";
import rawGradients from "./gradients.module.css";

const gradientsChecker = z.object({
  glow: z.string(),
  glowPink: z.string(),
  glowBlue: z.string(),
  glowConic: z.string(),
  glowSmall: z.string(),
  glowGray: z.string(),
});

const gradient = gradientsChecker.parse(rawGradients);

export function Gradient({
  width = 1000,
  height = 200,
  opacity,
  pink,
  blue,
  conic,
  gray,
  className,
  small,
}: {
  width?: number | string;
  height?: number | string;
  opacity?: number;
  pink?: boolean;
  blue?: boolean;
  conic?: boolean;
  gray?: boolean;
  className?: string;
  small?: boolean;
}): JSX.Element {
  return (
    <span
      className={cn(
        "absolute max-w-full rounded-full",
        gradient.glow,
        {
          [gradient.glowPink]: pink,
          [gradient.glowBlue]: blue,
          [gradient.glowConic]: conic,
          [gradient.glowSmall]: small,
          [gradient.glowGray]: gray,
        },
        className
      )}
      style={{
        width,
        height,
        opacity,
      }}
    />
  );
}
