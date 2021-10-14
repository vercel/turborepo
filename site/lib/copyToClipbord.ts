export const copyToClipboard = (str: string) => {
  const el = document.createElement('textarea') // Create a <textarea> element
  el.value = str // Set its value to the string that you want copied
  el.setAttribute('readonly', '') // Make it readonly to be tamper-proof
  el.style.position = 'absolute'
  el.style.left = '-9999px' // Move outside the screen to make it invisible
  document.body.appendChild(el) // Append the <textarea> element to the HTML document
  const selectedContent = document.getSelection()
  const selected =
    selectedContent && selectedContent.rangeCount > 0
      ? selectedContent.getRangeAt(0)
      : false
  el.select() // Select the <textarea> content
  document.execCommand('copy')
  document.body.removeChild(el) // Remove the <textarea> element
  if (selected && selectedContent) {
    // If a selection existed before copying
    selectedContent.removeAllRanges() // Unselect everything on the HTML document
    selectedContent.addRange(selected) // Restore the original selection
  }
}
