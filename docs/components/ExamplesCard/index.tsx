import { motion } from "framer-motion";
import { useState, useEffect } from "react";
import classNames from "classnames";
import copy from "copy-to-clipboard";
import { AnimatedIcon } from "./AnimatedIcon";

export function ExampleCard({
  name,
  description,
  slug,
  template,
}: {
  name: string;
  description: string;
  slug: string;
  template?: string;
}) {
  const [isHoveringStartBuilding, setIsHoveringStartBuilding] = useState(false);
  const [isHoveringDeployNow, setIsHoveringDeployNow] = useState(false);
  const [copiedStartBuildingCmd, setCopiedStartBuildingCmd] = useState(false);
  const [details, setDetails] = useState({
    icon: "github",
    text: `examples/${slug}`,
  });

  const onCopyStartBuildingCmd = () => {
    copy(`npx create-turbo -e ${slug}`);
    setCopiedStartBuildingCmd(true);
  };

  useEffect(() => {
    if (!copiedStartBuildingCmd) {
      return;
    }

    const timeout = setTimeout(() => {
      setCopiedStartBuildingCmd(false);
    }, 2000);

    return () => {
      clearTimeout(timeout);
    };
  }, [copiedStartBuildingCmd]);

  useEffect(() => {
    if (copiedStartBuildingCmd) {
      setDetails({
        icon: "prompt",
        text: `copied to clipboard`,
      });
    } else if (isHoveringStartBuilding) {
      setDetails({
        icon: "prompt",
        text: `npx create-turbo -e ${slug}`,
      });
    } else if (isHoveringDeployNow) {
      setDetails({
        icon: "vercel",
        text: `Deploy with Vercel`,
      });
    } else {
      setDetails({
        icon: "github",
        text: `examples/${slug}`,
      });
    }
  }, [
    isHoveringStartBuilding,
    isHoveringDeployNow,
    copiedStartBuildingCmd,
    slug,
  ]);

  return (
    <li className="col-span-1 md:col-span-2 lg:col-span-1 rounded-lg dark:bg-opacity-5 bg-white border-gray-500 text-white flex flex-col divide-y divide-[#dfdfdf] dark:divide-black shadow-lg">
      <a
        className="flex flex-col h-full gap-4 px-8 pt-8 cursor-pointer group"
        href={`https://github.com/vercel/turbo/tree/main/examples/${slug}`}
        rel="noreferrer"
        target="_blank"
      >
        <h3 className="text-lg font-semibold leading-6 tracking-tight text-black dark:text-white">
          {name}
        </h3>
        <span className="flex-1 text-base font-medium leading-7 text-gray-500 dark:text-gray-400">
          {description}
        </span>

        <div className="relative flex flex-row h-8 my-2 font-mono text-sm text-gray-500 dark:text-gray-400">
          <AnimatedIcon
            icon={details.icon}
            showCheck={copiedStartBuildingCmd}
          />
          <motion.span
            className={classNames("ml-3", {
              "group-hover:underline":
                !isHoveringStartBuilding && !isHoveringDeployNow,
              "text-gray-900 dark:text-gray-100":
                isHoveringStartBuilding || isHoveringDeployNow,
            })}
          >
            {details.text}
          </motion.span>
        </div>
      </a>
      <div className="flex w-full flex-row text-center text-base text-gray-500 dark:text-gray-400 justify-center self-end divide-x divide-[#dfdfdf] dark:divide-black">
        <button
          className="flex-1 p-4 rounded-none hover:text-gray-900 dark:hover:text-gray-100"
          onClick={onCopyStartBuildingCmd}
          onMouseEnter={() => {
            setIsHoveringStartBuilding(true);
          }}
          onMouseLeave={() => {
            setIsHoveringStartBuilding(false);
          }}
          type="button"
        >
          Start Building
        </button>
        {template ? (
          // eslint-disable-next-line jsx-a11y/anchor-is-valid -- This is an unfornate hack for right now. We need to fix this for a11y.
          <a
            className="flex-1 p-4 rounded-none hover:text-gray-900 dark:hover:text-gray-100"
            href={copiedStartBuildingCmd ? undefined : template}
            onMouseEnter={() => {
              setIsHoveringDeployNow(true);
            }}
            onMouseLeave={() => {
              setIsHoveringDeployNow(false);
            }}
            rel="noreferrer"
            target="_blank"
          >
            Deploy Now
          </a>
        ) : null}
      </div>
    </li>
  );
}
