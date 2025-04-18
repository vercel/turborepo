"use client";

import type { ReactElement } from "react";
import React, { useEffect, useState } from "react";
import { cn } from "@/components/cn";
import { users } from "./users";
import { Logo } from "./client-logo";
import { useTheme } from "next-themes";

interface LogoWrapperProps {
  className: string;
  children: ReactElement;
  staticWidth?: boolean;
}

function LogoWrapper({ className, children, staticWidth }: LogoWrapperProps) {
  if (!staticWidth) return children;
  return (
    <div
      className={cn("flex w-48 items-center justify-center lg:w-40", className)}
    >
      {children}
    </div>
  );
}

export function Clients({
  linked,
  staticWidth,
  companyList,
}: {
  linked?: boolean;
  staticWidth?: boolean;
  companyList?: string[];
}) {
  const [mounted, setMounted] = useState(false);
  const { theme } = useTheme();

  useEffect(() => {
    setMounted(true);
  }, []);

  // avoid hydration errors
  if (!mounted) return null;

  return users
    .filter((i) => (companyList ? companyList.includes(i.caption) : true))
    .map((user) => {
      const isDark = theme === "dark";
      const imgTheme = isDark ? "light" : "dark";

      return (
        <LogoWrapper
          className={isDark ? "hidden dark:flex" : "dark:hidden flex"}
          key={`${user.caption}-${imgTheme}`}
          staticWidth={staticWidth}
        >
          <Logo
            className={isDark ? "hidden dark:flex" : "dark:hidden flex"}
            isLink={linked ?? false}
            theme={imgTheme}
            user={user}
          />
        </LogoWrapper>
      );
    });
}
