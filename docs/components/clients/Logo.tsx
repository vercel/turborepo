import React from "react";
import cn from "classnames";
import Image from "next/future/image";
import { TurboUser } from "./users";

const DEFAULT_SIZE = {
  width: 100,
  height: 75,
};

export function Logo({
  user,
  theme,
  isLink,
}: {
  user: TurboUser;
  theme: "dark" | "light";
  isLink: boolean;
}) {
  const styles = {
    ...DEFAULT_SIZE,
    ...user.style,
  };
  const logo = (
    <Image
      src={user.image.replace(
        "/logos",
        theme === "light" ? "/logos/white" : "/logos/color"
      )}
      alt={`${user.caption}'s Logo`}
      width={styles.width}
      height={styles.height}
      priority={true}
      style={styles}
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
