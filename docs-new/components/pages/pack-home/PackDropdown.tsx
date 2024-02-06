import { useState, Fragment } from "react";
import { Listbox, Transition } from "@headlessui/react";
import type { BenchmarkNumberOfModules } from "./PackBenchmarks";

export function PackDropdown({
  onOptionSelected,
}: {
  onOptionSelected: (option: BenchmarkNumberOfModules) => void;
}) {
  const [selectedOption, setSelectedOption] =
    useState<BenchmarkNumberOfModules>("1000");

  const onSelect = (option: BenchmarkNumberOfModules) => {
    onOptionSelected(option);
    setSelectedOption(option);
  };

  return (
    <div className="relative">
      <Listbox onChange={onSelect} value={selectedOption}>
        <Listbox.Button className="flex w-24 pl-3 pr-2 py-2 gap-3 rounded !bg-[#fafafa] dark:!bg-[#111111] dark:hover:text-white hover:text-black dark:text-[#888888] text-[#666666] items-center justify-between transition-all text-sm leading-none font-medium m-0">
          {Number(selectedOption).toLocaleString()}
          <Arrow />
        </Listbox.Button>

        <Transition
          as={Fragment}
          leave="transition ease-in duration-100"
          leaveFrom="opacity-100"
          leaveTo="opacity-0"
        >
          <Listbox.Options className="absolute left-0 mt-2 w-full dark:bg-[#111111] bg-[#FAFAFA] rounded py-1 z-50 list">
            <Listbox.Option
              className={({ active }) =>
                `relative cursor-default select-none py-1 text-sm pl-3 text-gray-400 ${
                  active ? "bg-gray-800 text-gray-100" : "text-gray-900"
                }`
              }
              value="1000"
            >
              1000
            </Listbox.Option>
            <Listbox.Option
              className={({ active }) =>
                `relative cursor-default select-none py-1 text-sm pl-3 text-gray-400 ${
                  active ? "bg-gray-800 text-gray-100" : "text-gray-900"
                }`
              }
              value="5000"
            >
              5000
            </Listbox.Option>
            <Listbox.Option
              className={({ active }) =>
                `relative cursor-default select-none py-1 text-sm pl-3 text-gray-400 ${
                  active ? "bg-gray-800 text-gray-100" : "text-gray-900"
                }`
              }
              value="10000"
            >
              10000
            </Listbox.Option>
            <Listbox.Option
              className={({ active }) =>
                `relative cursor-default select-none py-1 text-sm pl-3 text-gray-400 ${
                  active ? "bg-gray-800 text-gray-100" : "text-gray-900"
                }`
              }
              value="30000"
            >
              30000
            </Listbox.Option>
          </Listbox.Options>
        </Transition>
      </Listbox>
    </div>
  );
}

function Arrow() {
  return (
    <svg
      fill="none"
      height="16"
      viewBox="0 0 16 16"
      width="16"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        d="M4 6L8 10L12 6"
        stroke="#666666"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.5"
      />
    </svg>
  );
}
