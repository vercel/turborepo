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
}): JSX.Element {
  const combinedClassname = cn(
    gradients.heroHeading,
    "font-bold tracking-[-0.03em] text-[24px] md:text-5xl lg:text-[40px] max-w-lg md:max-w-xl lg:max-w-4xl text-center",
    className
  );

  if (h1) {
    return <h1 className={combinedClassname}>{children}</h1>;
  }
  return <h2 className={combinedClassname}>{children}</h2>;
}

export function SectionHeader({
  children,
}: {
  children: React.ReactNode;
}): JSX.Element {
  return (
    <h2
      className={cn(
        gradients.heroHeading,
        "max-w-sm pb-1 text-center text-[32px] font-bold tracking-[-0.01em] md:max-w-md md:text-4xl lg:max-w-2xl lg:text-[40px]"
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
}): JSX.Element {
  const textClasses = hero
    ? "text-[20px] lg:text-xl"
    : "text-[16px] lg:text-[20px]";

  return (
    <p
      className={`font-mono leading-snug text-[#00000080] dark:text-[#FFFFFFB2] ${textClasses} xs:text-md max-w-md text-center  md:max-w-xl lg:max-w-[640px]`}
    >
      {children}
    </p>
  );
}
