import Image from "next/image";

export const Avatar = ({ name, picture, twitterUsername }) => {
  return (
    <div className="flex items-center">
      <Image
        src={picture}
        height={32}
        width={32}
        layout="fixed"
        priority={true}
        className="w-full rounded-full"
        alt={name}
      />
      <dl className="ml-2 text-sm font-medium leading-4 text-left whitespace-no-wrap">
        <dt className="sr-only">Name</dt>
        <dd className="text-gray-900 dark:text-white">{name}</dd>
        <dt className="sr-only">Twitter</dt>
        <dd>
          <a
            href={`https://twitter.com/${twitterUsername}`}
            className="text-xs text-blue-500 no-underline betterhover:hover:text-blue-600 betterhover:hover:underline"
            target="_blank"
            rel="noopener noreferrer"
          >
            @{/* */}
            {twitterUsername}
          </a>
        </dd>
      </dl>
    </div>
  );
};
