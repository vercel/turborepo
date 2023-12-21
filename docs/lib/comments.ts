import { useState } from "react";
import type { NextRouter } from "next/router";

const keyName = "vercel-comments";

export const useCommentsState = () => {
  // State to store our value
  const [storedValue] = useState(() => {
    if (typeof window !== "undefined") {
      // Get from local storage then
      // parse stored json or return initialValue
      return Boolean(window.localStorage.getItem(keyName));
    }
    // If server-side, return initialValue
    return false;
  });
  return storedValue;
};

export const setCommentsState = (router: NextRouter) => {
  if (localStorage.getItem(keyName)) {
    localStorage.removeItem(keyName);
  } else {
    localStorage.setItem(keyName, "1");
  }
  router.reload();
};
