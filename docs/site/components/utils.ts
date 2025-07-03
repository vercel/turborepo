function testPlatform(re: RegExp): boolean | undefined {
  // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition, eqeqeq -- Meaningfully different
  return window.navigator != null
    ? re.test(window.navigator.platform)
    : undefined;
}

export function isMac(): boolean | undefined {
  return testPlatform(/^Mac/);
}

export function isIPhone(): boolean | undefined {
  return testPlatform(/^iPhone/);
}

export function isIPad(): boolean | undefined {
  return (
    testPlatform(/^iPad/) ||
    // iPadOS 13 lies and says it's a Mac, but we can distinguish by detecting touch support.
    (isMac() && navigator.maxTouchPoints > 1)
  );
}

export function isIOS(): boolean | undefined {
  return isIPhone() || isIPad();
}

export function isApple(): boolean | undefined {
  return isMac() || isIPhone() || isIPad();
}
