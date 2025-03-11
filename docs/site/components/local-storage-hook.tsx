/* eslint-disable @typescript-eslint/no-unsafe-argument, @typescript-eslint/no-explicit-any, @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-return    */

"use client";

import type React from "react";
import {
  createContext,
  useContext,
  useState,
  useEffect,
  type ReactNode,
} from "react";

interface LocalStorageContextType {
  getItem: (key: string, initialValue: any) => any;
  setItem: (key: string, value: any) => void;
}

const LocalStorageContext = createContext<LocalStorageContextType | undefined>(
  undefined
);

export function LocalStorageProvider({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  const [storage, setStorage] = useState<Record<string, any>>({});

  useEffect(() => {
    // Initialize storage from localStorage
    if (typeof window !== "undefined") {
      const initialStorage: Record<string, any> = {};
      for (let i = 0; i < localStorage.length; i++) {
        const key = localStorage.key(i);
        if (key) {
          try {
            initialStorage[key] = JSON.parse(localStorage.getItem(key) || "");
          } catch {
            initialStorage[key] = localStorage.getItem(key);
          }
        }
      }
      setStorage(initialStorage);
    }
  }, []);

  const getItem = (key: string, initialValue: any): any => {
    return storage[key] !== undefined ? storage[key] : initialValue;
  };

  const setItem = (key: string, value: any): any => {
    setStorage((prev) => {
      const newStorage = { ...prev, [key]: value };
      if (typeof window !== "undefined") {
        localStorage.setItem(key, JSON.stringify(value));
      }
      return newStorage;
    });
  };

  return (
    <LocalStorageContext.Provider value={{ getItem, setItem }}>
      {children}
    </LocalStorageContext.Provider>
  );
}

export const useLocalStorage = <T,>(
  key: string | undefined,
  initialValue: T
): [T, (value: T | ((val: T) => T)) => void] => {
  const context = useContext(LocalStorageContext);
  if (!context) {
    throw new Error(
      "useLocalStorage must be used within a LocalStorageContext"
    );
  }

  const [storedValue, setStoredValue] = useState<T>(() => {
    if (key === undefined) {
      return initialValue;
    }
    return context.getItem(key, initialValue) as any;
  });

  const setValue = (value: T | ((val: T) => T)): void => {
    const valueToStore = value instanceof Function ? value(storedValue) : value;
    setStoredValue(valueToStore);
    if (key !== undefined) {
      context.setItem(key, valueToStore);
    }
  };

  useEffect(() => {
    if (key === undefined) return;
    setStoredValue(context.getItem(key, initialValue) as any);
  }, [context, key, initialValue]);

  return [storedValue, setValue];
};
