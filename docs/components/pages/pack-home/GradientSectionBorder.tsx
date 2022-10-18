import cn from "classnames";
import Image from "next/future/image";
import { FadeIn } from "./FadeIn";
import { Gradient } from "./Gradient";
import gradients from "./gradients.module.css";

export function GradientSectionBorder({
  children,
  hexBottomOffset,
}: {
  children: React.ReactNode;
  hexBottomOffset?: number;
}) {
  return (
    <section className={cn("relative overflow-hidden")}>
      <FadeIn noVertical viewTriggerOffset>
        <span
          className={cn(
            "w-full absolute white h-[1px] top-0 opacity-25",
            gradients.gradientSectionBorderDivider
          )}
        />
        <span
          className={cn(
            gradients.gradientSectionBorder,
            gradients.gradientSectionBorderLeft,
            "dark:opacity-35 opacity-[0.15]"
          )}
        />
        <span
          className={cn(
            gradients.gradientSectionBorder,
            gradients.gradientSectionBorderRight,
            "dark:opacity-35 opacity-[0.15]"
          )}
        />
        <span
          className={cn(
            "absolute -z-50 top-[-180px] left-[-80px]",
            gradients.hexagonWrapper
          )}
        >
          <Image
            alt="Hexagons"
            src={`/images/docs/pack/left-hexagons.svg`}
            width={474}
            height={542}
          />
          <div
            className={cn(
              "absolute right-0 w-full h-full top-0",
              gradients.hexagonOverlay
            )}
          />
        </span>
      </FadeIn>
      <div
        className={cn(
          "absolute -z-50 -bottom-16 right-[0px] flex align-center items-center",
          gradients.hexagonWrapper
        )}
        style={{ bottom: hexBottomOffset }}
      >
        <Image
          alt="Hexagons"
          src={`/images/docs/pack/right-hexagons.svg`}
          width={231}
          height={342}
        />
        <Gradient
          width={500}
          height={300}
          pink
          className="dark:opacity-[0.1] opacity-[0.05]"
        />
      </div>
      {children}
    </section>
  );
}
