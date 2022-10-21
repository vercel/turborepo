import cn from "classnames";
import { FadeIn } from "./FadeIn";
import gradients from "../home-shared/gradients.module.css";

export function GradientSectionBorder({
  children,
}: {
  children: React.ReactNode;
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
      </FadeIn>
      {children}
    </section>
  );
}
