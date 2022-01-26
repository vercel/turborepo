import React from "react";

const renderComponent = (ComponentOrNode, props, functionOnly) => {
  if (!ComponentOrNode) return null;
  if (typeof ComponentOrNode === "function") {
    if (functionOnly) return ComponentOrNode(props);
    return <ComponentOrNode {...props} />;
  }
  return ComponentOrNode;
};

export default renderComponent;
