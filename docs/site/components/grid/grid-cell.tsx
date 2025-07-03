import { cn } from "../cn";

export interface GridItemProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
  colSpan?: {
    sm?: number;
    md?: number;
    lg?: number;
    xl?: number;
  };
  rowSpan?: {
    sm?: number;
    md?: number;
    lg?: number;
    xl?: number;
  };
}

export function GridCell({
  children,
  colSpan,
  rowSpan,
  className,
  ...props
}: GridItemProps) {
  const colSpanClass = cn(
    // @ts-expect-error -- This works as expected
    colSpan?.sm && `col-span-${colSpan.sm}`,
    colSpan?.md && `md:col-span-${colSpan.md}`,
    colSpan?.lg && `lg:col-span-${colSpan.lg}`,
    colSpan?.xl && `xl:col-span-${colSpan.xl}`
  );

  const rowSpanClass = cn(
    // @ts-expect-error -- This works as expected
    rowSpan?.sm && `row-span-${rowSpan.sm}`,
    rowSpan?.md && `md:row-span-${rowSpan.md}`,
    rowSpan?.lg && `lg:row-span-${rowSpan.lg}`,
    rowSpan?.xl && `xl:row-span-${rowSpan.xl}`
  );

  return (
    <div
      className={cn(
        colSpanClass,
        rowSpanClass,
        "border-0 border-gray-200 p-6 xs:p-12",
        className
      )}
      {...props}
    >
      {children}
    </div>
  );
}
