import React from "react";
import cn from "classnames";
import { users } from "./users";
import { Logo } from "./Logo";

export function Clients({
  linked,
  staticWidth,
  companyList,
}: {
  linked?: boolean;
  staticWidth?: boolean;
  companyList?: string[];
}) {
  const showcaseDark = [];
  const showcaseLight = [];

  const LogoWrapper = ({ className, children }) => {
    if (!staticWidth) return children;
    return (
      <div className={cn("w-48 flex items-center justify-center", className)}>
        {children}
      </div>
    );
  };

  users
    .filter((i) => (companyList ? companyList.includes(i.caption) : true))
    .forEach((user) => {
      if (user.pinned) {
        showcaseDark.push(
          <LogoWrapper className="flex dark:hidden">
            <Logo
              user={user}
              key={`${user.caption}-dark`}
              theme={"dark"}
              isLink={linked}
            />
          </LogoWrapper>
        );
        showcaseLight.push(
          <LogoWrapper className="hidden dark:flex">
            <Logo
              user={user}
              key={`${user.caption}-light`}
              theme={"light"}
              isLink={linked}
            />
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
