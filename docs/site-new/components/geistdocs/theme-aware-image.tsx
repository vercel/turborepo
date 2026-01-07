import type { StaticImageData } from "next/image";
import Image from "next/image";
import { cn } from "@/lib/utils";

interface ImageAttrs {
  src: StaticImageData;
  alt: string;
  className?: string;
  props?: {
    width: number;
    height: number;
  };
}

interface ThemeAwareImageProps {
  className?: string;
  light: ImageAttrs;
  dark: ImageAttrs;
}

export function ThemeAwareImage({
  className,
  light,
  dark
}: ThemeAwareImageProps) {
  const Images = (
    <>
      <Image
        alt={dark.alt}
        className={cn("hidden dark:block", dark.className)}
        src={dark.src}
        width={dark.props?.width}
        height={dark.props?.height}
      />
      <Image
        alt={light.alt}
        className={cn("block dark:hidden", light.className)}
        src={light.src}
        width={light.props?.width}
        height={light.props?.height}
      />
    </>
  );

  if (className) {
    return <div className={className}>{Images}</div>;
  }

  return Images;
}
