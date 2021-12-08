// BFS traverse the page map tree
export default function traverse(pageMap, matcher) {
  for (let i = 0; i < pageMap.length; i++) {
    if (matcher(pageMap[i])) {
      return pageMap[i];
    }
  }
  for (let i = 0; i < pageMap.length; i++) {
    if (pageMap[i].children) {
      const matched = traverse(pageMap[i].children, matcher);
      if (matched) {
        return matched;
      }
    }
  }
  return null;
}
