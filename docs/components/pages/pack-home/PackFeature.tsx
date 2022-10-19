import Image from "next/future/image";

export function PackFeature({
  name,
  description,
  iconDark,
  iconLight,
}: {
  iconDark: string;
  iconLight: string;
  name: string;
  description: string;
}) {
  return (
    <div className="flex flex-col p-8 gap-5 no-underline text-black dark:text-white  relative overflow-hidden rounded-xl box-border border dark:border-neutral-800">
      <Image
        src={iconDark}
        width={64}
        height={64}
        aria-hidden="true"
        alt=""
        className="dark:block hidden"
      />
      <Image
        src={iconLight}
        width={64}
        height={64}
        aria-hidden="true"
        alt=""
        className="dark:hidden block"
      />
      <div className="flex flex-col gap-2">
        <h4 className="m-0 font-bold font-space-grotesk leading-5 text-gray-900 dark:text-white">
          {name}
        </h4>

        <p className="m-0 leading-6 opacity-70">{description}</p>
      </div>
    </div>
  );
}
