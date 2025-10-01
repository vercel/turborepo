import type { ReactNode } from "react";
import { cn } from "#components/cn.ts";

export type BadgeProps = React.ComponentProps<"span"> & {
  children: ReactNode;
  className?: string;
};

export function Badge(props: BadgeProps): JSX.Element {
  const { children, className, ...rest } = props;

  return (
    <span
      className={cn(
        "text-background my-auto box-border inline-flex h-5 shrink-0 items-center justify-center whitespace-nowrap rounded-lg bg-gradient-to-r from-[#d74a41] to-[#407aeb] px-2 align-middle text-xs font-bold capitalize tabular-nums",
        className
      )}
      {...rest}
    >
      {children}
    </span>
  );
}
