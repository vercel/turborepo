import React from "react";
import { cn } from "#components/cn.ts";
import type { ImageFigureProps } from "./image-figure";
import type { ThemedImageProps } from "./themed-image";
import { ThemedImage } from "./themed-image";

export type ThemedImageFigureProps = Omit<ImageFigureProps, "src"> &
  ThemedImageProps;

export function ThemedImageFigure(
  props: ThemedImageFigureProps
): React.ReactNode {
  const {
    caption,
    margin = 40,
    captionSpacing = null,
    shadow = false,
    borderRadius = false,
    ...rest
  } = props;

  return (
    <figure
      className="-mx-4 block text-center sm:-mx-4 md:-mx-7 lg:-mx-12"
      style={{ marginTop: `${margin}px`, marginBottom: `${margin}px` }}
    >
      <div
        className={cn(
          "border-box relative inline-block max-w-full overflow-hidden text-[0px]",
          borderRadius ? "rounded-md" : "",
          shadow ? "shadow-lg" : ""
        )}
      >
        {}
        <ThemedImage {...rest} />
      </div>
      {caption ? (
        <figcaption
          className="m-0 text-center text-xs text-gray-500"
          style={captionSpacing ? { marginTop: captionSpacing } : {}}
        >
          {caption}
        </figcaption>
      ) : null}
    </figure>
  );
}
