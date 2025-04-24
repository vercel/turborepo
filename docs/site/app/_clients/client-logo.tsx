import React from "react";
import Image from "next/image";
import { cn } from "@/components/cn";
import type { TurboUser } from "./users";

const DEFAULT_SIZE = {
  width: 100,
  height: 75,
};

export function Logo({
  user,
  theme,
  isLink,
  className,
}: {
  user: TurboUser;
  theme: "dark" | "light";
  isLink: boolean;
  className?: string;
}): JSX.Element {
  const styles = {
    ...DEFAULT_SIZE,
    ...user.style,
  };
  let numericWidth: number;
  let numericHeight: number;
  if (typeof styles.width === "number") {
    numericWidth = styles.width;
  }
  if (typeof styles.height === "number") {
    numericHeight = styles.height;
  }
  const logo = (
    <Image
      alt={`${user.caption}'s Logo`}
      className={cn("mx-8", className)}
      // biome-ignore lint/style/noNonNullAssertion: Ignored using `--suppress`
      height={numericHeight!}
      priority
      src={user.image.replace(
        "/logos",
        theme === "light" ? "/logos/white" : "/logos/color"
      )}
      style={styles}
      // biome-ignore lint/style/noNonNullAssertion: Ignored using `--suppress`
      width={numericWidth!}
    />
  );

  if (isLink) {
    return (
      <a
        className="item-center flex justify-center"
        href={user.infoLink}
        rel="noopener noreferrer"
        target="_blank"
      >
        {logo}
      </a>
    );
  }

  return logo;
}
