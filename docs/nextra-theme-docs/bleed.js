import React from 'react'
import cn from 'classnames'

const Bleed = (
  {
    full,
    children,
  }
) => {
  return (
    <div className={cn('bleed relative mt-6 -mx-6 md:-mx-8 2xl:-mx-24', { full })}>
      {children}
    </div>
  )
};

export default Bleed;
