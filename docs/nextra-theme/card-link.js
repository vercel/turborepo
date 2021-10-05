import React from 'react'
import Link from 'next/link'

function CardLink({ href, children }) {
  return (
    <div>
      <Link href={href}>
        <a className="block font-bold text-gray-900 dark:text-white no-underline  rounded-xl ring-1 ring-black ring-opacity-5 shadow-sm p-6">
          {children}
        </a>
      </Link>
    </div>
  )
}

export default CardLink
