import React from "react";
import cn from "classnames";

interface Image {
  src: string;
  alt: string;
  className?: string;
}

interface Props {
  className?: string;
  light: Image;
  dark: Image;
}

export default function ThemeAwareImage({
  className,
  light,
  dark,
  ...other
}: Props) {
  const Images = (
    <>
      <img
        className={cn("hidden dark:block", dark.className)}
        alt={dark.alt}
        src={dark.src}
        {...other}
      />
      <img
        className={cn("dark:hidden block", light.className)}
        alt={light.alt}
        src={light.src}
        {...other}
      />
    </>
  );

  if (className) {
    return <div className={className}>{Images}</div>;
  }

  return Images;
}
