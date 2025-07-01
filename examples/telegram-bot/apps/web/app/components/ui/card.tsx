"use client";
import React from "react";
import { cn } from "../../lib/utils";

interface CardProps {
  children: React.ReactNode;
  className?: string;
  hover?: boolean;
}

export const Card = React.forwardRef<HTMLDivElement, CardProps>(
  ({ children, className }, ref) => {
    return (
      <div
        ref={ref}
        className={cn(
          "bg-white/25 border border-gray-200/25 rounded-xl p-6 shadow-sm backdrop-blur-sm",
          "dark:bg-gray-900/25 dark:border-gray-700/25",
          className
        )}
      >
        {children}
      </div>
    );
  }
);

Card.displayName = "Card";

interface StatCardProps {
  title: string;
  value: string | number;
  className?: string;
}

export const StatCard = ({ title, value, className }: StatCardProps) => {
  return (
    <Card className={className}>
      <h3 className="text-sm font-semibold text-gray-600 dark:text-gray-400 uppercase tracking-wide mb-2">
        {title}
      </h3>
      <p className="text-3xl font-bold text-gray-900 dark:text-white font-mono">
        {value}
      </p>
    </Card>
  );
};
