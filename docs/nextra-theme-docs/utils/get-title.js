import innerText from 'react-innertext'

export function getTitle(headings) {
  const titleEl = headings.find(child => child.type === 'h1')
  return titleEl ? innerText(titleEl.props.children) : null
}
