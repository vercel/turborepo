import { useRouter } from "next/router";
import cn from "classnames";
import { PlusSmIcon } from "@heroicons/react/outline";
import { getCommentsState, setCommentsState } from "../lib/comments";

export function CommentsButton() {
  const comments = getCommentsState();
  const router = useRouter();

  return (
    <button
      className={cn(
        "transition-all duration-700 w-6 h-6 -translate-x-0.5 -translate-y-0.5 rounded-tl-none rounded-full border-2 border-white",
        comments
          ? "bg-blue-500 hover:bg-gray-500"
          : "bg-gray-500 hover:bg-blue-500"
      )}
      onClick={() => {
        setCommentsState(router);
      }}
      type="button"
    >
      <PlusSmIcon className="w-4 h-4 translate-x-[.1rem] stroke-2" />
    </button>
  );
}

// function CommentsButton({ text, isActive }) {
//   const router = useRouter();
//   const classes =
//     "py-1 transition-colors duration-300 inline-block w-[50px] cursor-pointer hover:text-black dark:hover:text-white";

//   const conditionalClasses = {
//     "text-black dark:text-white": Boolean(isActive),
//   };

//   return (
//     <button
//       className={cn(
//         "block p-1.5 px-2 border rounded transition-all text-gray-300 text-sm hover:bg-white hover:text-black"
//         // comments ? "text-blue-300 border-blue-500" : null
//       )}
//       onClick={() => {
//         setCommentsState(router);
//       }}
//       type="button"
//     >
//       {text}
//     </button>
//   );
// }

// export function CommentsSwitch() {
//   const comments = getCommentsState();
//   return (
//     <div className="relative flex items-center justify-between p-2 text-xl group">
//       <span
//         className={cn(
//           "flex h-[34px] w-[100px] flex-shrink-0 items-center rounded-[8px] border border-[#dedfde] dark:border-[#333333] p-1 duration-300 ease-in-out",
//           "after:h-[24px] after:w-[44px] after:rounded-md dark:after:bg-[#333333] after:shadow-sm after:duration-300 after:border dark:after:border-[#333333] after:border-[#666666]/100 after:bg-gradient-to-b after:from-[#3286F1] after:to-[#C33AC3] after:opacity-20 dark:after:opacity-100 dark:after:bg-none",
//           "indeterminate:after:hidden"
//           // {
//           //   "after:hidden": !site,
//           //   "after:translate-x-[46px]": site === "pack",
//           // }
//         )}
//       />

//       <span
//         className={cn(
//           "z-50 absolute p-1 text-sm flex justify-between text-center w-[100px] text-[#666666] dark:text-[#888888]"
//           // { "hover:text-black dark:hover:text-white": site }
//         )}
//       />
//       <CommentsButton isActive={comments} text="yes" />
//       <CommentsButton isActive={!comments} text="no" />
//     </div>
//   );
// }
