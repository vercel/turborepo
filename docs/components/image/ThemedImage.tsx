import React from "react";
import Image from "next/image";

interface ImageAttrs {
  height: number;
  width: number;
  source: string;
}

export interface ThemedImageProps {
  title: string;
  dark: ImageAttrs;
  light: ImageAttrs;
  priority?: boolean;
}

export function ThemedImage({
  title,
  light,
  dark,
  priority = false,
}: ThemedImageProps) {
  return (
    <>
      <div className="block w-full dark:hidden">
        <Image
          alt={title}
          height={light.height}
          priority={priority}
          src={light.source}
          width={light.width}
        />
      </div>
      <div className="hidden w-full dark:block">
        <Image
          alt={title}
          height={dark.height}
          priority={priority}
          src={dark.source}
          width={dark.width}
        />
      </div>
    </>
  );
}
