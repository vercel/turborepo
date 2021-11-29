import React from 'react'

const renderComponent = (ComponentOrNode, props) => {
  if (!ComponentOrNode) return null
  if (typeof ComponentOrNode === 'function') {
    return <ComponentOrNode {...props} />
  }
  return ComponentOrNode
}

export default renderComponent
