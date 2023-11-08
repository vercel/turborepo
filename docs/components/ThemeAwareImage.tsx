import React from "react";
import cn from "classnames";
import type { ImageProps } from "next/image";
import Image from "next/image";

interface ImageAttrs {
  src: string;
  alt: string;
  className?: string;
  props?: ImageProps;
}

interface Props {
  className?: string;
  light: ImageAttrs;
  dark: ImageAttrs;
}

export default function ThemeAwareImage({
  className,
  light,
  dark,
  ...other
}: Props) {
  const Images = (
    <>
      <Image
        alt={dark.alt}
        className={cn("hidden dark:block", dark.className)}
        src={dark.src}
        {...dark.props}
        {...other}
      />
      <Image
        alt={light.alt}
        className={cn("dark:hidden block", light.className)}
        src={light.src}
        {...light.props}
        {...other}
      />
    </>
  );

  if (className) {
    return <div className={className}>{Images}</div>;
  }

  return Images;
}
