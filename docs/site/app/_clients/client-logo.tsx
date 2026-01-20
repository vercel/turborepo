import type { ReactElement } from "react";
import Image from "next/image";
import { cn } from "@/lib/utils";
import type { TurboUser } from "./users";

const DEFAULT_SIZE = {
  width: 100,
  height: 75
};

export function Logo({
  user,
  theme,
  isLink,
  className
}: {
  user: TurboUser;
  theme: "dark" | "light";
  isLink: boolean;
  className?: string;
}): ReactElement {
  const styles = {
    ...DEFAULT_SIZE,
    ...user.style
  };

  // Initialize with default values
  let numericWidth = DEFAULT_SIZE.width;
  let numericHeight = DEFAULT_SIZE.height;

  if (typeof styles.width === "number") {
    numericWidth = styles.width;
  }
  if (typeof styles.height === "number") {
    numericHeight = styles.height;
  }

  // Use white logos as base, invert for light mode to get dark grayscale logos
  const logo = (
    <Image
      alt={`${user.caption}'s Logo`}
      className={cn(
        "mx-8",
        // Light mode: invert white logos to dark
        theme === "dark" && "invert",
        className
      )}
      height={numericHeight}
      priority
      src={user.imageWhite}
      style={styles}
      width={numericWidth}
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
