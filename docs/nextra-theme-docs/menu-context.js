import { useContext, createContext } from 'react'

export const MenuContext = createContext(false)
export default function useMenuContext() {
  return useContext(MenuContext)
}
