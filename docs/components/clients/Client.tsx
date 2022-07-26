import React from "react";
import Image from "next/image";

interface ClientProps {
  name: string;
  image: string;
  className?: string;
  theme: string;
  style?: React.CSSProperties;
}

export const Client = React.memo<ClientProps>(
  ({ name, image, style, theme, ...rest }) => (
    <span title={name} {...rest}>
      <Image
        src={image.replace(
          "/logos",
          theme == "dark" ? "/logos/white" : "/logos/color"
        )}
        alt={name}
        width={style?.width ?? style?.maxWidth ?? 100}
        height={style?.height ?? 75}
        loading="lazy"
        className="inline"
      />
    </span>
  )
);

Client.displayName = "Client";
