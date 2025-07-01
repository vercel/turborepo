"use client";
import React from "react";
import { cn } from "../../lib/utils";
import Link from "next/link";

interface ButtonProps {
  children: React.ReactNode;
  variant?: "primary" | "secondary";
  size?: "sm" | "md" | "lg";
  className?: string;
  href?: string;
  onClick?: (e: React.MouseEvent<HTMLButtonElement>) => void;
  disabled?: boolean;
}

const buttonVariants = {
  primary:
    "bg-gradient-to-br from-blue-300 to-blue-800 text-white shadow-lg hover:shadow-xl hover:-translate-y-0.5",
  secondary:
    "bg-white/25 border border-gray-200/25 text-gray-900 hover:bg-gray-50/25 hover:border-blue-500/25 hover:-translate-y-0.5 dark:bg-gray-900/25 dark:border-gray-700/25 dark:text-white dark:hover:bg-gray-800/25 backdrop-blur-sm",
};

const buttonSizes = {
  sm: "h-10 px-4 text-sm",
  md: "h-12 px-6 text-base",
  lg: "h-14 px-8 text-lg",
};

export const Button = ({
  children,
  variant = "primary",
  size = "md",
  className,
  href,
  onClick,
  disabled = false,
}: ButtonProps) => {
  const baseClasses = cn(
    "inline-flex items-center justify-center rounded-xl font-semibold transition-all duration-200 min-w-48 gap-2",
    buttonVariants[variant],
    buttonSizes[size],
    disabled && "opacity-50 cursor-not-allowed",
    className
  );

  if (href) {
    return (
      <Link href={href} className={baseClasses}>
        {children}
      </Link>
    );
  }

  return (
    <button className={baseClasses} onClick={onClick} disabled={disabled}>
      {children}
    </button>
  );
};
