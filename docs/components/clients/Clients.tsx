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

  function LogoWrapper({ className, children }) {
    if (!staticWidth) return children;
    return (
      <div
        className={cn(
          "w-48 lg:w-40 flex items-center justify-center",
          className
        )}
      >
        {children}
      </div>
    );
  }

  users
    .filter((i) => (companyList ? companyList.includes(i.caption) : true))
    .forEach((user) => {
      if (user.pinned) {
        showcaseDark.push(
          <LogoWrapper
            className="flex dark:hidden"
            key={`${user.caption}-dark`}
          >
            <Logo isLink={linked} theme="dark" user={user} />
          </LogoWrapper>
        );
        showcaseLight.push(
          <LogoWrapper
            className="hidden dark:flex"
            key={`${user.caption}-light`}
          >
            <Logo isLink={linked} theme="light" user={user} />
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
