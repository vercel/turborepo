import React from 'react'

const Callout = ({
  children,
  background = 'bg-orange-100 dark:bg-gray-700 dark:bg-opacity-25 dark:text-white',
  emoji = '💡'
}) => {
  return (
    <div className={`${background} flex rounded-lg callout mt-6`}>
      <div className="pl-3 pr-2 py-2 select-none text-xl">{emoji}</div>
      <div className="pr-4 py-2">{children}</div>
    </div>
  )
}

// https://www.notion.so/Callout-blocks-5b2638247b54447eb2e21145f97194b0
export default Callout
