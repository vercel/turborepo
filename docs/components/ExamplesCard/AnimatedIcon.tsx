import type { SVGProps } from "react";
import { useState, useEffect } from "react";
import { AnimatePresence, motion } from "framer-motion";

type SVGPaths = Record<string, SVGProps<SVGPathElement>>;

const AnimatedPaths: SVGPaths = {
  prompt: {
    d: "M103.5 439.875L258.75 284.625L103.5 129.375",
  },
  check: {
    d: "M73.0002 364.165L252.952 489.952L540 84.5",
  },
};

const StaticPaths: SVGPaths = {
  vercel: {
    d: "M310 72L378.75 191.2L447.5 310.399L516.25 429.599L585 548.799H447.5H310H172.5H35L103.75 429.599L172.5 310.399L241.25 191.2L310 72Z",
  },
  github: {
    fillRule: "evenodd",
    clipRule: "evenodd",
    d: "M310.228 39C158.034 39 35 163.026 35 316.464C35 439.114 113.832 542.936 223.193 579.681C236.866 582.444 241.874 573.711 241.874 566.365C241.874 559.933 241.423 537.884 241.423 514.911C164.862 531.452 148.919 481.836 148.919 481.836C136.615 449.679 118.384 441.415 118.384 441.415C93.3254 424.417 120.209 424.417 120.209 424.417C148.006 426.255 162.591 452.898 162.591 452.898C187.194 495.157 226.838 483.217 242.787 475.866C245.063 457.949 252.358 445.547 260.105 438.658C199.041 432.225 134.795 408.339 134.795 301.761C134.795 271.442 145.724 246.637 163.042 227.345C160.31 220.456 150.738 191.97 165.78 153.843C165.78 153.843 189.019 146.491 241.418 182.324C263.852 176.25 286.987 173.16 310.228 173.134C333.466 173.134 357.156 176.353 379.032 182.324C431.436 146.491 454.675 153.843 454.675 153.843C469.717 191.97 460.14 220.456 457.407 227.345C475.182 246.637 485.66 271.442 485.66 301.761C485.66 408.339 421.414 431.763 359.894 438.658C369.922 447.385 378.575 463.92 378.575 490.106C378.575 527.314 378.125 557.176 378.125 566.36C378.125 573.711 383.139 582.444 396.806 579.687C506.167 542.93 584.999 439.114 584.999 316.464C585.449 163.026 461.965 39 310.228 39Z",
  },
};

export function AnimatedIcon({
  icon,
  showCheck,
}: {
  icon: string;
  showCheck?: boolean;
}) {
  const [showCheckInternal, setShowCheckInternal] = useState(showCheck);

  useEffect(() => {
    if (!showCheck) {
      return;
    }

    setShowCheckInternal(true);
    const timeout = setTimeout(() => {
      setShowCheckInternal(false);
    }, 1500);

    return () => {
      clearTimeout(timeout);
    };
  }, [showCheck]);

  if (icon === "vercel" || icon === "github") {
    return (
      <motion.svg
        animate={{
          opacity: 1,
        }}
        className="fill-black dark:fill-white"
        fill="none"
        height="18"
        initial={{
          opacity: 0,
        }}
        key={icon}
        transition={{
          duration: 0.2,
        }}
        viewBox="0 0 621 621"
        width="18"
        xmlns="http://www.w3.org/2000/svg"
      >
        <path {...StaticPaths[icon]} />
      </motion.svg>
    );
  }

  return (
    <AnimatePresence>
      <motion.svg
        animate={{ opacity: 1 }}
        className="stroke-black dark:stroke-white"
        exit={{ opacity: 0 }}
        fill="none"
        height="18"
        initial={{ opacity: 0 }}
        key={icon}
        transition={{ duration: 0.2 }}
        viewBox="0 0 621 621"
        width="18"
        xmlns="http://www.w3.org/2000/svg"
      >
        {/* prompt > or check */}
        <motion.path
          animate={{
            d: showCheckInternal
              ? AnimatedPaths.check.d
              : AnimatedPaths.prompt.d,
          }}
          d={AnimatedPaths[showCheckInternal ? "check" : "prompt"].d}
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth="50"
          transition={{
            duration: 0.15,
          }}
        />
        {/* prompt: bottom line */}
        <motion.path
          animate={{
            opacity: showCheckInternal ? 0 : 1,
          }}
          d="M310.5 491.625H517.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth="50"
          transition={{
            duration: 0.15,
          }}
        />
      </motion.svg>
    </AnimatePresence>
  );
}
