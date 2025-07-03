import { cn } from "../cn";

export interface GridProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
  columns?: {
    sm?: number;
    md?: number;
    lg?: number;
    xl?: number;
  };
  gap?: "none" | "small" | "medium" | "large";
}

export function Grid({
  children,
  columns = {
    sm: 1,
    md: 2,
    lg: 3,
    xl: 4,
  },
  gap = "none",
  className,
  ...props
}: GridProps) {
  const gapClass = {
    none: "gap-0",
    small: "gap-2",
    medium: "gap-4",
    large: "gap-8",
  }[gap];

  const gridClass = cn(
    "grid w-full",
    gapClass,
    // @ts-expect-error -- This works as expected
    columns.sm && `grid-cols-${columns.sm}`,
    columns.md && `md:grid-cols-${columns.md}`,
    columns.lg && `lg:grid-cols-${columns.lg}`,
    columns.xl && `xl:grid-cols-${columns.xl}`,
    className
  );

  return (
    <div className={gridClass} {...props}>
      {children}
    </div>
  );
}
