import { motion, useAnimationControls } from "framer-motion";
import { useState, useEffect } from "react";
import classNames from "classnames";
import copy from "copy-to-clipboard";
import { GitHubIcon } from "../Icons";
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

    return () => clearTimeout(timeout);
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
  }, [isHoveringStartBuilding, isHoveringDeployNow, copiedStartBuildingCmd]);

  return (
    <li className="col-span-1 md:col-span-2 lg:col-span-1 rounded-lg dark:bg-opacity-5 bg-white border-gray-500 text-white flex flex-col divide-y divide-[#dfdfdf] dark:divide-black shadow-lg">
      <a
        className="flex flex-col group px-8 pt-8 gap-4 h-full cursor-pointer"
        href={`https://github.com/vercel/turbo/tree/main/examples/${slug}`}
        target="_blank"
        rel="noreferrer"
      >
        <h3 className="text-lg font-semibold leading-6 tracking-tight">
          <span className="bg-gradient-to-r from-[#a44e9c] to-[#ff1e57] bg-clip-text text-transparent">
            {name}
          </span>
        </h3>
        <span className="flex-1 text-base font-medium leading-7 text-gray-500 dark:text-gray-400">
          {description}
        </span>

        <div className="font-mono text-sm text-gray-500 dark:text-gray-400 flex flex-row relative h-8 my-2">
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
          className="rounded-none flex-1 p-4 hover:text-gray-900 dark:hover:text-gray-100"
          onClick={onCopyStartBuildingCmd}
          onMouseEnter={() => setIsHoveringStartBuilding(true)}
          onMouseLeave={() => setIsHoveringStartBuilding(false)}
        >
          Start Building
        </button>
        {template && (
          <a
            target="_blank"
            rel="noreferrer"
            className="rounded-none flex-1 p-4 hover:text-gray-900 dark:hover:text-gray-100"
            href={copiedStartBuildingCmd ? undefined : template}
            onMouseEnter={() => setIsHoveringDeployNow(true)}
            onMouseLeave={() => setIsHoveringDeployNow(false)}
          >
            Deploy Now
          </a>
        )}
      </div>
    </li>
  );
}
