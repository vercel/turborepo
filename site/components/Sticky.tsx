import * as React from 'react'

import cn from 'classnames'

export interface StickyProps {
  offset?: number
  shadow?: boolean
  className?: string
}

export const Sticky: React.FC<StickyProps> = ({
  offset,
  children,
  shadow,
  className,
}) => {
  return (
    <div
      style={{ top: offset || 0 }}
      className={cn({ shadow }, 'z-20', className)}
    >
      {children}

      <style jsx>{`
        div {
          position: sticky;
        }
        div.shadow {
          box-shadow: rgba(0, 0, 0, 0.06) 0px 6px 20px;
        }
      `}</style>
    </div>
  )
}

Sticky.displayName = 'Sticky'
