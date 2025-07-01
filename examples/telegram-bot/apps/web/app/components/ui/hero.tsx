"use client";
import React from "react";
import { cn } from "../../lib/utils";

interface HeroProps {
  title: string;
  subtitle?: string;
  children?: React.ReactNode;
  className?: string;
  whiteText?: boolean;
}

export const Hero = ({
  title,
  subtitle,
  children,
  className,
  whiteText = false,
}: HeroProps) => {
  return (
    <div className={cn("my-8 text-center", className)}>
      <h1
        className={cn(
          "text-2xl md:text-5xl font-bold mb-4 font-bitcount",
          whiteText
            ? "text-white"
            : "bg-gradient-to-br from-blue-100 to-blue-500 bg-clip-text text-transparent"
        )}
      >
        {title}
      </h1>
      {subtitle && (
        <p className="text-xl text-gray-600 dark:text-gray-300 mb-6">
          {subtitle}
        </p>
      )}
      {children}
    </div>
  );
};
