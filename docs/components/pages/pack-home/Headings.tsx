import cn from "classnames";
import gradients from "./gradients.module.css";

export function HeroText({ children }: { children: React.ReactNode }) {
  return (
    <h1
      className={cn(
        gradients.heroHeading,
        "font-semibold font-wide tracking-[-0.01em] leading-none text-[40px] md:text-5xl lg:text-[64px] max-w-lg md:max-w-xl lg:max-w-3xl text-center text-transparent"
      )}
    >
      {children}
    </h1>
  );
}

export function SectionHeader({ children }: { children: React.ReactNode }) {
  return (
    <h2
      className={cn(
        gradients.heroHeading,
        "font-semibold font-wide tracking-[-0.01em] leading-tight text-[32px] md:text-4xl lg:text-[40px] max-w-sm md:max-w-md lg:max-w-2xl text-center text-transparent"
      )}
    >
      {children}
    </h2>
  );
}

export function SectionSubtext({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="leading-snug dark:text-[#FFFFFFB2] text-[#00000080] text-[20px] lg:text-2xl max-w-sm md:max-w-md lg:max-w-2xl text-center">
      {children}
    </h3>
  );
}
