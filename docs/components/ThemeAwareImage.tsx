import React from "react";
import cn from "classnames";
import Image, { ImageProps } from "next/image";

interface Image {
  src: string;
  alt: string;
  className?: string;
  props?: ImageProps;
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
      <Image
        className={cn("hidden dark:block", dark.className)}
        alt={dark.alt}
        src={dark.src}
        {...dark.props}
        {...other}
      />
      <Image
        className={cn("dark:hidden block", light.className)}
        alt={light.alt}
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
