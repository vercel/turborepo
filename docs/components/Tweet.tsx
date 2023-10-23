import Image from "next/image";

function TweetLink({ href, children }) {
  return (
    <a
      className="inline-block text-[#35ACDF]"
      href={href}
      rel="noopener noreferrer"
      target="_blank"
    >
      {children}
    </a>
  );
}

export function Mention({ children }) {
  return (
    <TweetLink href={`https://x.com/${children.replace("@", "")}`}>
      {children}
    </TweetLink>
  );
}

export default function Tweet({ username, name, avatar, date, children }) {
  return (
    <div className="flex p-4 bg-white rounded-md shadow-xl dark:bg-opacity-10">
      <div className="flex-shrink-0 mr-4">
        <Image
          alt={`${name} X avatar`}
          className="w-12 h-12 rounded-full"
          height={42}
          src={avatar}
          width={42}
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
