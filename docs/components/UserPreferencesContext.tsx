import React from "react";
import { useTabGroupChoice } from "./useTabGroupChoice";

export interface UserPreferencesContextProps {
  tabGroupChoices: { readonly [groupId: string]: string };
  setTabGroupChoices: (groupId: string, newChoice: string) => void;
}

const UserPreferencesContext = React.createContext<UserPreferencesContextProps>(
  undefined as any as UserPreferencesContextProps
);

export function useUserPreferences(): UserPreferencesContextProps {
  const context = React.useContext<UserPreferencesContextProps>(
    UserPreferencesContext as any
  );
  if (context == null) {
    throw new Error(
      "`useUserPreferencesContext` is used outside of  UserPreferencesContext Component."
    );
  }
  return context;
}

export function UserPreferencesProvider(props: {
  children: React.ReactNode;
}): JSX.Element {
  const { tabGroupChoices, setTabGroupChoices } = useTabGroupChoice();

  return (
    <UserPreferencesContext.Provider
      value={{
        tabGroupChoices,
        setTabGroupChoices,
      }}
    >
      {props.children}
    </UserPreferencesContext.Provider>
  );
}
