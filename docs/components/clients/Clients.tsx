import React from "react";
import { users } from "./users";
import { Logo } from "./Logo";

export function Clients({ linked }: { linked?: boolean }) {
  const showcaseDark = [];
  const showcaseLight = [];
  users.forEach((user) => {
    if (user.pinned) {
      showcaseDark.push(
        <Logo
          user={user}
          key={`${user.caption}-dark`}
          theme={"dark"}
          isLink={linked}
        />
      );
      showcaseLight.push(
        <Logo
          user={user}
          key={`${user.caption}-light`}
          theme={"light"}
          isLink={linked}
        />
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
