const titleType = ['h1', 'h2', 'h3', 'h4', 'h5', 'h6']

export function getHeadings(children) {
  return children?.filter?.(child => titleType.includes(child.type)) || []
}
