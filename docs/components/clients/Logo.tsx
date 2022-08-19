import React from "react";
import cn from "classnames";
import Image from "next/future/image";
import { TurboUser } from "./users";

export function Logo({
  user,
  theme,
  isLink,
}: {
  user: TurboUser;
  theme: "dark" | "light";
  isLink: boolean;
}) {
  const logo = (
    <Image
      src={user.image.replace(
        "/logos",
        theme === "light" ? "/logos/white" : "/logos/color"
      )}
      alt={user.caption}
      width={user.style?.width ?? 100}
      height={user.style?.height ?? 75}
      priority={true}
      className={cn("mx-8", {
        "hidden dark:inline": theme !== "dark",
        "dark:hidden inline": theme === "dark",
      })}
    />
  );

  if (isLink) {
    return (
      <a
        href={user.infoLink}
        target="_blank"
        rel="noopener noreferrer"
        className={cn("flex justify-center item-center", {
          "hidden dark:flex": theme !== "dark",
          "dark:hidden flex": theme === "dark",
        })}
      >
        {logo}
      </a>
    );
  }

  return logo;
}
