import * as React from "react";

/**
 * Use a layout effect in the browser and a passive effect during server rendering.
 * See: https://react.dev/reference/react/useLayoutEffect
 */
const useIsomorphicLayoutEffect =
  typeof window !== "undefined" ? React.useLayoutEffect : React.useEffect;

export { useIsomorphicLayoutEffect };
