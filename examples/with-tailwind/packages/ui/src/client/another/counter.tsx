'use client';

import { useState } from 'react'

export function Counter() {
  const [count, setCount] = useState(0)

  return (
    <div>
      <p>You clicked {count} times</p>
      <button className="ui-bg-neutral-500 ui-p-4 ui-rounded-lg ui-pointer-cursor ui-border-2 border-solid ui-border-neutral-300 hover:ui-bg-white hover:ui-text-neutral-500 ui-m-4 ui-mx-auto" onClick={() => setCount(count + 1)}>Click Me!!</button>
    </div>
  )
}