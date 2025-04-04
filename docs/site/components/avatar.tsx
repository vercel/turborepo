import Image from "next/image";
import type { AuthorDetails } from "./team";

export function Avatar({
  name,
  picture,
  xUsername,
}: AuthorDetails): JSX.Element {
  return (
    <div className="not-prose flex flex-shrink-0 items-center md:justify-start">
      <div className="h-[32px] w-[32px]">
        <Image
          alt={name}
          className="w-full rounded-full"
          height={32}
          priority
          src={picture}
          title={name}
          width={32}
        />
      </div>
      <dl className="whitespace-no-wrap ml-2 text-accent text-left text-sm leading-4">
        <dt className="sr-only">Name</dt>
        <dd className="text-foreground mb-0.5">{name}</dd>
        {xUsername ? (
          <>
            <dt className="sr-only">X</dt>
            <dd>
              <a
                className="text-foreground/70 text-xs no-underline"
                href={`https://x.com/${xUsername}`}
                rel="noopener noreferrer"
                target="_blank"
              >
                {`@${xUsername}`}
              </a>
            </dd>
          </>
        ) : null}
      </dl>
    </div>
  );
}
