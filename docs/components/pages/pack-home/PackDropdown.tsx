import { useState } from "react";
import { motion } from "framer-motion";
import { BenchmarkNumberOfModules } from "./PackBenchmarks";

export function PackDropdown({
  onOptionSelected,
}: {
  onOptionSelected: (option: BenchmarkNumberOfModules) => void;
}) {
  const [isOpen, setIsOpen] = useState(false);
  const [selectedOption, setSelectedOption] =
    useState<BenchmarkNumberOfModules>("1000");

  const onSelect = (option: BenchmarkNumberOfModules) => {
    onOptionSelected(option);
    setIsOpen(false);
    setSelectedOption(option);
  };

  return (
    <div className="relative">
      <button
        onClick={() => setIsOpen(!isOpen)}
        type="button"
        className={`flex w-24 pl-3 pr-2 py-2 gap-3 border rounded dark:border-[#333333] dark:hover:border-white border-[#EAEAEA] hover:border-black dark:hover:text-white hover:text-black dark:text-[#888888] text-[#666666] items-center justify-between transition-all`}
      >
        <p className="text-sm leading-none font-medium m-0 ">
          {Number(selectedOption).toLocaleString()}
        </p>

        <Arrow />
      </button>
      {isOpen && (
        <motion.ol
          initial={{ y: -8, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          transition={{ duration: 0.2, ease: "easeOut" }}
          className="absolute left-0 mt-2 w-full dark:bg-[#000] bg-[#fff] border dark:border-[#333333] rounded py-1 z-50"
        >
          <BenchmarkOption value="1000" onSelect={onSelect} />
          <BenchmarkOption value="5000" onSelect={onSelect} />
          <BenchmarkOption value="10000" onSelect={onSelect} />
          <BenchmarkOption value="30000" onSelect={onSelect} />
        </motion.ol>
      )}
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
