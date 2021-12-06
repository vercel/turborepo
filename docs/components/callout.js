import React from "react";
import LightBulbIcon from "@heroicons/react/solid/LightBulbIcon";
import WarningIcon from "@heroicons/react/solid/ExclamationIcon";
import ErrorIcon from "@heroicons/react/solid/ExclamationCircleIcon";
import InformationCircleIcon from "@heroicons/react/solid/InformationCircleIcon";

const themes = {
  default: {
    classes:
      "bg-orange-100 text-orange-800 dark:text-orange-300 dark:bg-orange-200 dark:bg-opacity-10",
    icon: <WarningIcon className="w-5 h-5 mt-1" />,
  },
  info: {
    classes:
      "bg-blue-100 text-blue-800 dark:text-blue-300 dark:bg-blue-200 dark:bg-opacity-10",
    icon: <InformationCircleIcon className="w-5 h-5 mt-1" />,
  },
  idea: {
    classes:
      "bg-gray-100 text-gray-800 dark:text-gray-300 dark:bg-gray-200 dark:bg-opacity-10",
    icon: <LightBulbIcon className="w-5 h-5 mt-1" />,
  },
  error: {
    classes:
      "bg-red-200 text-red-900 dark:text-red-200 dark:bg-red-600 dark:bg-opacity-30",
    icon: <ErrorIcon className="w-5 h-5 mt-1" />,
  },
  default: {
    classes:
      "bg-orange-100 text-orange-800 dark:text-orange-300 dark:bg-orange-200 dark:bg-opacity-10",
    icon: <WarningIcon className="w-5 h-5 mt-1" />,
  },
};

export default function Callout({ children, type = "default", icon }) {
  return (
    <div className={`${themes[type].classes} flex rounded-lg callout mt-6`}>
      <div
        className="py-2 pl-3 pr-2 text-xl select-none"
        style={{
          fontFamily:
            '"Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol"',
        }}
      >
        {icon || themes[type].icon}
      </div>
      <div className="py-2 pr-4">{children}</div>
    </div>
  );
}
