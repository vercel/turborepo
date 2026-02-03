import { memo, type ReactNode } from "react";
import { Handle, Position, type NodeProps } from "reactflow";

export interface TurboNodeData {
  title: string;
  icon?: ReactNode;
  subtitle?: string;
}

function TurboNodeComponent({ data }: NodeProps<TurboNodeData>) {
  return (
    <div className="wrapper gradient">
      <div className="inner">
        <div className="body">
          {data.icon && <div className="icon">{data.icon}</div>}
          <div>
            <div className="title">{data.title}</div>
            {data.subtitle && <div className="subtitle">{data.subtitle}</div>}
          </div>
        </div>
        <Handle type="target" position={Position.Left} />
        <Handle type="source" position={Position.Right} />
      </div>
    </div>
  );
}

export const TurboNode = memo(TurboNodeComponent);
