import Alert from './alert'
import { Footer2 } from './Footer2'

interface LayoutProps {
  preview?: boolean
  dark?: boolean
  stripe?: boolean
  showCta: boolean
  children: React.ReactNode
}

export const Layout = ({
  preview,
  stripe = true,
  children,
  showCta = true,
  dark = true,
}: LayoutProps) => {
  return (
    <>
      <div
        className={'dark:bg-gray-900 dark:bg-opacity-5 bg-white min-h-screen'}
      >
        {stripe ? (
          <div className="h-2 w-full bg-gradient-to-r from-blue-500 to-red-600"></div>
        ) : (
          <div className="h-2 w-full bg-gray-900"></div>
        )}
        <Alert preview={preview} />
        <main>{children}</main>
      </div>
      <Footer2 showCta={showCta} />
    </>
  )
}
