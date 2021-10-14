import { useEffect, useState } from 'react'
import { useIsSSR } from '@react-aria/ssr'

export function useMediaQuery(query: string) {
  let supportsMatchMedia =
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
  let [matches, setMatches] = useState(() =>
    supportsMatchMedia ? window.matchMedia(query).matches : false
  )

  useEffect(() => {
    if (!supportsMatchMedia) {
      return
    }

    let mq = window.matchMedia(query)
    let onChange = (evt: any) => {
      setMatches(evt.matches)
    }

    mq.addListener(onChange)
    return () => {
      mq.removeListener(onChange)
    }
  }, [supportsMatchMedia, query])

  // If in SSR, the media query should never match. Once the page hydrates,
  // this will update and the real value will be returned.
  let isSSR = useIsSSR()
  return isSSR ? false : matches
}
