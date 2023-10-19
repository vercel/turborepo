import { useState, Fragment } from "react";
import { Listbox, Transition } from "@headlessui/react";
import { BenchmarkNumberOfModules } from "./PackBenchmarks";

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
      <Listbox value={selectedOption} onChange={onSelect}>
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
              value="1000"
              className={({ active }) =>
                `relative cursor-default select-none py-1 text-sm pl-3 text-gray-400 ${
                  active ? "bg-gray-800 text-gray-100" : "text-gray-900"
                }`
              }
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

function BenchmarkOption({
  value,
  onSelect,
}: {
  value: BenchmarkNumberOfModules;
  onSelect: (value: string) => void;
}) {
  return (
    <div
      className="flex pl-3 py-2 items-center justify-between cursor-pointer transition-all dark:text-[#888888] dark:hover:text-white text-[#666666] hover:text-[#000]"
      onClick={() => onSelect(value)}
    >
      <p className="text-sm leading-none font-medium m-0">
        {Number(value).toLocaleString()}
      </p>
    </div>
  );
}

function Arrow() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        d="M4 6L8 10L12 6"
        stroke="#666666"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
