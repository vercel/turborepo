import { useRouter } from "next/router";
import { getCommentsState, setCommentsState } from "../lib/comments";

export function CommentsButton() {
  const comments = getCommentsState();
  const router = useRouter();

  return (
    <button
      className=""
      onClick={() => {
        setCommentsState(router);
      }}
      type="button"
    >
      Comments: {comments ? "On" : "Off"}
    </button>
  );
}
