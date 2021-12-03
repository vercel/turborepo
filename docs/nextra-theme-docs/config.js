import React from 'react'

export const ThemeConfigContext = React.createContext({})
export const useConfig = () => React.useContext(ThemeConfigContext)
