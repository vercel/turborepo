'use client';

import { useState } from 'react'

export function Counters() {
  const [countA, setCountA] = useState(0)
  const [countB, setCountB] = useState(0)
  const [countC, setCountC] = useState(0)

  return (
    <div>
      <p>You clicked A {countA} times,</p>
      <p>You clicked B {countB} times,</p>
      <p>You clicked C {countC} times.</p>
      <div className="ui-flex space-x-2">
        <button className="ui-bg-neutral-600 ui-p-4 ui-rounded-lg ui-pointer-cursor ui-border-2 border-solid ui-border-neutral-300 hover:ui-bg-white hover:ui-text-neutral-600 ui-m-4 ui-mx-auto" onClick={() => setCountA(countA + 1)}>A</button>
        <button className="ui-bg-purple-900 ui-p-4 ui-rounded-lg ui-pointer-cursor ui-border-2 border-solid ui-border-neutral-300 hover:ui-bg-white hover:ui-text-red-900 ui-m-4 ui-mx-auto" onClick={() => setCountB(countB + 1)}>B</button>
        <button className="ui-bg-red-900 ui-p-4 ui-rounded-lg ui-pointer-cursor ui-border-2 border-solid ui-border-neutral-300 hover:ui-bg-white hover:ui-text-red-900 ui-m-4 ui-mx-auto" onClick={() => setCountC(countC + 1)}>C</button>
      </div>
    </div>
  )
}