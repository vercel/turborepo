import cn from "classnames";
import { useRouter } from "next/router";

export function useTurboSite(): "pack" | "repo" | undefined {
  const { pathname } = useRouter();

  if (pathname.startsWith("/repo")) {
    return "repo";
  }

  if (pathname.startsWith("/pack")) {
    return "pack";
  }

  return undefined;
}

function SiteSwitcher() {
  const router = useRouter();
  const site = useTurboSite();

  const handleChange = () => {
    // special cases where we know we can map 1:1 between pack and repo
    if (router.pathname.endsWith("/docs")) {
      router.push(site === "repo" ? "/pack/docs" : "/repo/docs");
      return;
    }

    // fallback to just redirecting to the root
    router.push(site === "repo" ? "/pack" : "/repo");
  };

  if (!site) {
    return null;
  }

  return (
    <label
      className={cn(
        "group relative flex items-center justify-between p-2 text-xl",
        { "cursor-pointer": site, "cursor-not-allowed": !site }
      )}
    >
      <input
        tabIndex={0}
        disabled={!site}
        onChange={handleChange}
        checked={site === "pack"}
        type="checkbox"
        className="peer absolute left-1/2 h-full w-full -translate-x-1/2 appearance-none rounded-md"
      />
      <span
        className={cn(
          "flex h-[34px] w-[100px] flex-shrink-0 items-center rounded-[8px] border border-[#dedfde] dark:border-[#333333] p-1 duration-300 ease-in-out",
          "after:h-[24px] after:w-[44px] after:rounded-md dark:after:bg-[#333333] after:shadow-sm after:duration-300 after:border dark:after:border-[#333333] after:border-[#666666]/100 after:bg-gradient-to-b after:from-[#3286F1] after:to-[#C33AC3] after:opacity-20 dark:after:opacity-100 dark:after:bg-none",
          "indeterminate:after:hidden",
          "group-hover:after:translate-x-[4px] peer-checked:after:translate-x-[46px] group-hover:peer-checked:after:translate-x-[42px]",
          {
            "after:hidden": !site,
          }
        )}
      />
      <span className="z-50 absolute p-1 text-sm flex justify-between text-center w-[100px] text-[#666666] dark:text-[#888888]">
        <span
          className={cn(
            "py-1 transition-colors duration-300 inline-block w-[50px]",
            {
              "text-black dark:text-white": site === "repo",
            }
          )}
        >
          Repo
        </span>
        <span
          className={cn(
            "inline-block w-[50px] py-1 transition-colors duration-300",
            {
              "text-black dark:text-white": site === "pack",
            }
          )}
        >
          Pack
        </span>
      </span>
    </label>
  );
}

export default SiteSwitcher;
