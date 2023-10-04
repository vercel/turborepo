import React from "react";
import Image from "next/image";

export interface Image {
  height: number;
  width: number;
  source: string;
}

export interface ThemedImageProps {
  title?: string;
  dark?: Image;
  light?: Image;
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
          src={light.source}
          width={light.width}
          height={light.height}
          priority={priority}
        />
      </div>
      <div className="hidden w-full dark:block">
        <Image
          alt={title}
          src={dark.source}
          width={dark.width}
          height={dark.height}
          priority={priority}
        />
      </div>
    </>
  );
}
