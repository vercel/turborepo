import type { NextRouter } from "next/router";

const keyName = "vercel-comments";

export const getCommentsState = () => {
  if (typeof window !== "undefined") {
    return Boolean(localStorage.getItem(keyName));
  }

  // Always false on server
  return false;
};

export const setCommentsState = (router: NextRouter) => {
  if (localStorage.getItem(keyName)) {
    localStorage.removeItem(keyName);
  } else {
    localStorage.setItem(keyName, "1");
  }
  router.reload();
};
