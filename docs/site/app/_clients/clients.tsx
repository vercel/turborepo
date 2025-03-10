import type { ReactElement, JSX } from "react";
import React from "react";
import cn from "classnames";
import { users } from "./users";
import { Logo } from "./client-logo";

interface LogoWrapperProps {
  className: string;
  children: JSX.Element;
  staticWidth?: boolean;
}

function LogoWrapper({
  className,
  children,
  staticWidth,
}: LogoWrapperProps): JSX.Element {
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
}): JSX.Element {
  const showcaseDark: ReactElement[] = [];
  const showcaseLight: ReactElement[] = [];

  users
    .filter((i) => (companyList ? companyList.includes(i.caption) : true))
    .forEach((user) => {
      if (user.pinned) {
        showcaseDark.push(
          <LogoWrapper
            className="flex dark:hidden"
            key={`${user.caption}-dark`}
            staticWidth={staticWidth}
          >
            <Logo isLink={linked ?? false} theme="dark" user={user} />
          </LogoWrapper>
        );
        showcaseLight.push(
          <LogoWrapper
            className="hidden dark:flex"
            key={`${user.caption}-light`}
            staticWidth={staticWidth}
          >
            <Logo isLink={linked ?? false} theme="light" user={user} />
          </LogoWrapper>
        );
      }
    });

  return (
    <>
      {showcaseDark}
      {showcaseLight}
    </>
  );
}
