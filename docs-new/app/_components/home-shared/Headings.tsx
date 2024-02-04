import cn from "classnames";
import gradients from "./gradients.module.css";

export function HeroText({
  children,
  className,
  h1,
}: {
  children: React.ReactNode;
  className?: string;
  h1?: boolean;
}) {
  const combinedClassname = cn(
    gradients.heroHeading,
    "font-extrabold tracking-[-0.04em] leading-none text-[40px] md:text-5xl lg:text-[80px] max-w-lg md:max-w-xl lg:max-w-4xl text-center text-transparent",
    className
  );

  if (h1) {
    return <h1 className={combinedClassname}>{children}</h1>;
  }
  return <h2 className={combinedClassname}>{children}</h2>;
}

export function SectionHeader({ children }: { children: React.ReactNode }) {
  return (
    <h2
      className={cn(
        gradients.heroHeading,
        "font-bold tracking-[-0.01em] pb-1 text-[32px] md:text-4xl lg:text-[40px] max-w-sm md:max-w-md lg:max-w-2xl text-center text-transparent"
      )}
    >
      {children}
    </h2>
  );
}

export function SectionSubtext({
  hero,
  children,
}: {
  hero?: boolean;
  children: React.ReactNode;
}) {
  const textClasses = hero
    ? "text-[20px] lg:text-xl"
    : "text-[16px] lg:text-[20px]";

  return (
    <p
      className={`font-space-grotesk leading-snug dark:text-[#FFFFFFB2] text-[#00000080] ${textClasses} max-w-md md:max-w-xl lg:max-w-[640px] text-center`}
    >
      {children}
    </p>
  );
}
