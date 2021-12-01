import * as React from "react";
import { useNumberFieldState } from "@react-stately/numberfield";
import { useNumberField } from "@react-aria/numberfield";
import { useLocale } from "@react-aria/i18n";
import { useButton } from "@react-aria/button";
import { AriaNumberFieldProps } from "@react-types/numberfield";

export function NumberInput(
  props: AriaNumberFieldProps & { gradient?: boolean }
) {
  let { locale } = useLocale();
  let state = useNumberFieldState({ ...props, locale });
  let inputRef = React.useRef<HTMLInputElement>(null);
  let {
    labelProps,
    groupProps,
    inputProps,
    incrementButtonProps,
    decrementButtonProps,
  } = useNumberField(props, state, inputRef);
  let incRef = React.useRef<HTMLDivElement>(null);
  let decRef = React.useRef<HTMLDivElement>(null);
  let { buttonProps: incrementProps } = useButton(incrementButtonProps, incRef);
  let { buttonProps: decrementProps } = useButton(decrementButtonProps, decRef);
  let inputClassName = props.gradient
    ? "rounded-md text-xl font-medium bg-gradient-to-r from-blue-500 to-red-600 text-white border-transparent border-none focus:border-transparent focus:ring focus:ring-blue-200 focus:ring-opacity-50 w-full"
    : props.isDisabled
    ? "rounded-md text-xl font-medium bg-white bg-opacity-5 text-white border-transparent focus:border-transparent focus:ring focus:ring-blue-200 focus:ring-opacity-50 w-full"
    : "rounded-l-md text-xl font-medium bg-white bg-opacity-5 text-blue-400 border-transparent focus:border-transparent focus:ring focus:ring-blue-200 focus:ring-opacity-50 w-full";
  let labelClassName = props.isDisabled
    ? props.gradient
      ? "inline-block text-lg font-medium bg-clip-text text-transparent bg-gradient-to-r from-blue-500 to-red-500"
      : "text-gray-400 text-lg font-medium"
    : "text-blue-400 text-lg font-medium";
  return (
    <div>
      <label className={labelClassName} {...labelProps}>
        {props.label}
      </label>
      <div
        className="flex mt-1 border rounded-md shadow-sm bg-opacity-5 dark:border-transparent "
        {...groupProps}
      >
        <div className="relative flex items-stretch flex-grow focus-within:z-10">
          <input
            className={inputClassName}
            {...inputProps}
            ref={inputRef}
            style={{ opacity: props.isDisabled && !props.gradient ? 0.5 : 1 }}
          />
        </div>
        {!props.isDisabled && (
          <>
            <div className="relative flex flex-col -ml-px text-sm font-medium border-l divide-y text-dark rounded-r-md bg-gray-50 bg-opacity-10 dark:bg-opacity-5 dark:border-black dark:divide-black ">
              <button
                className="flex-1 px-4 text-gray-700 dark:text-gray-500 dark:hover:bg-black dark:hover:bg-opacity-20 betterhover:hover:bg-gray-100 rounded-tr-md focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
                {...incrementProps}
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  viewBox="0 0 20 20"
                  fill="currentColor"
                  height={14}
                  width={14}
                >
                  <path
                    fillRule="evenodd"
                    d="M14.707 12.707a1 1 0 01-1.414 0L10 9.414l-3.293 3.293a1 1 0 01-1.414-1.414l4-4a1 1 0 011.414 0l4 4a1 1 0 010 1.414z"
                    clipRule="evenodd"
                  />
                </svg>
              </button>
              <button
                className="flex-1 px-4 dark:text-gray-500 dark:hover:bg-black dark:hover:bg-opacity-20 betterhover:hover:bg-gray-100 rounded-br-md focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
                {...decrementProps}
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  viewBox="0 0 20 20"
                  fill="currentColor"
                  height={14}
                  width={14}
                >
                  <path
                    fillRule="evenodd"
                    d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z"
                    clipRule="evenodd"
                  />
                </svg>
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
