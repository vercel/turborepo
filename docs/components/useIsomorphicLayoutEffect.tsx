import * as React from "react";

const useIsomorphicLayoutEffect =
  typeof window !== "undefined" ? React.useLayoutEffect : React.useEffect;

/* eslint-disable-next-line import/no-default-export -- TODO: Fix ESLint Error (#13355) */
export default useIsomorphicLayoutEffect;
