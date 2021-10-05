import '../styles.css'
import '../nextra-theme/styles.css'

export default function Nextra({ Component, pageProps }: any) {
  return (
    <>
      <Component {...pageProps} />
    </>
  )
}
