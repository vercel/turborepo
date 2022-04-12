import React, { useState, Children, ReactElement, useEffect } from "react";
import { useUserPreferences } from "./UserPreferencesContext";
import clsx from "classnames";

const keys = {
  left: 37,
  right: 39,
  tab: 9,
};

type Props = {
  block?: boolean;
  children: ReactElement<{ value: string }>[];
  defaultValue?: string;
  values: { value: string; label: string }[];
  groupId?: string;
};

function Tabs(props: Props): JSX.Element {
  const { block, children, defaultValue, values, groupId } = props;
  const { tabGroupChoices, setTabGroupChoices } = useUserPreferences();
  const [selectedValue, setSelectedValue] = useState(defaultValue);
  const [keyboardPress, setKeyboardPress] = useState(false);

  if (groupId != null) {
    const relevantTabGroupChoice = tabGroupChoices[groupId];
    if (
      relevantTabGroupChoice != null &&
      relevantTabGroupChoice !== selectedValue &&
      values.some((value) => value.value === relevantTabGroupChoice)
    ) {
      setSelectedValue(relevantTabGroupChoice);
    }
  }

  const changeSelectedValue = (newValue: string) => {
    setSelectedValue(newValue);
    if (groupId != null) {
      setTabGroupChoices(groupId, newValue);
    }
  };

  const tabRefs: (HTMLLIElement | null)[] = [];

  const focusNextTab = (tabs: any[], target: any) => {
    const next = tabs.indexOf(target) + 1;

    if (!tabs[next]) {
      tabs[0].focus();
    } else {
      tabs[next].focus();
    }
  };

  const focusPreviousTab = (tabs: any[], target: any) => {
    const prev = tabs.indexOf(target) - 1;

    if (!tabs[prev]) {
      tabs[tabs.length - 1].focus();
    } else {
      tabs[prev].focus();
    }
  };

  const handleKeydown = (tabs: any[], target: any[], event: any) => {
    switch (event.keyCode) {
      case keys.right:
        focusNextTab(tabs, target);
        break;
      case keys.left:
        focusPreviousTab(tabs, target);
        break;
      default:
        break;
    }
  };

  const handleKeyboardEvent = (event: any) => {
    if (event.metaKey || event.altKey || event.ctrlKey) {
      return;
    }

    setKeyboardPress(true);
  };

  const handleMouseEvent = () => {
    setKeyboardPress(false);
  };

  useEffect(() => {
    window.addEventListener("keydown", handleKeyboardEvent);
    window.addEventListener("mousedown", handleMouseEvent);
  }, []);

  return (
    <div>
      <div className="border-b border-gray-200 dark:border-gray-800">
        <ul
          role="tablist"
          aria-orientation="horizontal"
          className={clsx("!list-none -mb-px flex space-x-8", {
            "tabs--block": block,
          })}
        >
          {values.map(({ value, label }) => (
            <li
              role="tab"
              tabIndex={0}
              aria-selected={selectedValue === value}
              className={clsx("!mt-0 !mb-0 hover:cursor-default", {
                "whitespace-no-wrap py-4 px-1 border-b-2 border-transparent font-medium text-sm leading-5 text-gray-500 dark:text-gray-300 hover:text-gray-700 hover:border-gray-300 focus:outline-none focus:text-gray-700 focus:border-gray-300":
                  selectedValue !== value,
                "whitespace-no-wrap py-4 px-1 border-b-2 border-blue-500 dark:border-blue-500 font-medium text-sm leading-5 text-blue-600 dark:text-blue-400 focus:outline-none dark:focus:text-blue-500 focus:text-blue-800 focus:border-blue-700":
                  selectedValue === value,
                "tabs__item--active": selectedValue === value,
              })}
              style={keyboardPress ? {} : { outline: "none" }}
              key={value}
              ref={(tabControl) => tabRefs.push(tabControl)}
              onKeyDown={(event: React.BaseSyntheticEvent) => {
                handleKeydown(tabRefs, event.target, event);
                handleKeyboardEvent(event);
              }}
              onFocus={() => changeSelectedValue(value)}
              onClick={() => {
                changeSelectedValue(value);
                setKeyboardPress(false);
              }}
              onPointerDown={() => setKeyboardPress(false)}
            >
              {label}
            </li>
          ))}
        </ul>
      </div>
      <div role="tabpanel" className="my-6">
        {
          Children.toArray(children).filter(
            (child) =>
              (child as ReactElement<{ value: string }>).props.value ===
              selectedValue
          )[0]
        }
      </div>
    </div>
  );
}

function TabItem(props: { readonly children: React.ReactNode }): JSX.Element {
  return <div>{props.children}</div>;
}

Tabs.TabItem = TabItem;

export default Tabs;
