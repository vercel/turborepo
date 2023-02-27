import React, { memo, ReactNode } from "react";
import { Handle, NodeProps, Position } from "reactflow";
import FunctionIcon from "./FunctionIcon";

export type TurboNodeData = {
  title: string;
  icon?: ReactNode;
  subline?: string;
};

const TurboNode = ({ data }: NodeProps<TurboNodeData>) => {
  return (
    <>
      <div className="cloud gradient">
        <div>
          <FunctionIcon />
        </div>
      </div>
      <div className="wrapper gradient">
        <div className="inner">
          <div className="body">
            {data.icon && <div className="icon">{data.icon}</div>}
            <div>
              <div className="title">{data.title}</div>
              {data.subline && <div className="subline">{data.subline}</div>}
            </div>
          </div>
          <Handle type="target" position={Position.Left} />
          <Handle type="source" position={Position.Right} />
        </div>
      </div>
    </>
  );
};

export default memo(TurboNode);
