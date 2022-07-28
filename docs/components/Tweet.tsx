import Image from "next/image";

function TweetLink({ href, children }) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className="inline-block text-[#35ACDF]"
    >
      {children}
    </a>
  );
}

export function Mention({ children }) {
  return (
    <TweetLink href={`https://twitter.com/${children.replace("@", "")}`}>
      {children}
    </TweetLink>
  );
}

export default function Tweet({ url, username, name, avatar, date, children }) {
  return (
    <div className="flex p-4 bg-white rounded-md shadow-xl dark:bg-opacity-10">
      <div className="flex-shrink-0 mr-4">
        <Image
          className="w-12 h-12 rounded-full"
          width={42}
          height={42}
          src={avatar}
          alt={`${name} twitter avatar`}
        />
      </div>
      <div>
        <div className="flex items-center space-x-1 text-sm">
          <h4 className="font-medium dark:text-white">{name}</h4>
          <div className="truncate dark:text-gray-400">@{username}</div>
          <div className="dark:text-gray-500 md:hidden xl:block">• {date}</div>
        </div>
        <div className="mt-1 text-sm dark:text-gray-200">{children}</div>
      </div>
    </div>
  );
}
