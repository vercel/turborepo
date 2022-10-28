import Link from "next/link";
import { ReactNode } from "react";
import cn from "classnames";

type LinkButtonProps = {
  href: string;
  children: ReactNode;
  size?: "sm";
};

export default function LinkButton({ href, children, size }: LinkButtonProps) {
  return (
    <Link href={href}>
      <a
        className={cn(
          "px-4 py-2 text-gray-600 hover:text-black no-underline bg-gray-100 rounded-full dark:bg-opacity-5 dark:text-gray-300 dark:hover:text-white",
          { "text-sm": size === "sm" }
        )}
      >
        {children}
      </a>
    </Link>
  );
}
