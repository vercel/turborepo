import { useRouter } from "next/router";
import { useCommentsState, setCommentsState } from "../lib/comments";

export function CommentsButton() {
  const comments = useCommentsState();
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
