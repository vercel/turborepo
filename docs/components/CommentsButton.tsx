import { useRouter } from "next/router";
import cn from "classnames";
import { PlusSmIcon } from "@heroicons/react/outline";
import { getCommentsState, setCommentsState } from "../lib/comments";

export function CommentsButton() {
  const comments = getCommentsState();
  const router = useRouter();

  return (
    // Tailwind preflight breaks button styles when type is set: https://github.com/shuding/nextra/issues/1403
    // eslint-disable-next-line react/button-has-type -- Can't set it here because it messes up background color. There's a style that sets the background to transparent that I believe is coming from deep within Nextra.
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
    >
      <PlusSmIcon className="w-4 h-4 translate-x-[.1rem] stroke-2" />
    </button>
  );
}
