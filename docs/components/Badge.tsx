import cn from "classnames";

import type { ReactNode } from "react";

export type BadgeProps = React.ComponentProps<"span"> & {
  children: ReactNode;
  className?: string;
};

export default function Badge(props: BadgeProps) {
  const { children, className, ...rest } = props;

  return (
    <span
      className={cn(
        "dark:text-black text-white inline-flex items-center justify-center shrink-0 box-border rounded-lg capitalize whitespace-nowrap font-bold tabular-nums h-5 px-2 text-xs bg-gradient-to-r from-[#d74a41] to-[#407aeb] align-middle",
        className
      )}
      {...rest}
    >
      {children}
    </span>
  );
}
