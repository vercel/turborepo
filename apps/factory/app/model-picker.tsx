"use client";

import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";

interface ModelOption {
  readonly id: string;
  readonly label: string;
}

interface ModelPickerProps {
  readonly disabled: boolean;
  readonly models: readonly ModelOption[];
  readonly onValueChange: (value: string) => void;
  readonly value: string;
}

export function ModelPickerEnhancer() {
  const [select, setSelect] = useState<HTMLSelectElement | null>(null);
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    const element = document.querySelector<HTMLSelectElement>(
      "select#workspace-model"
    );
    if (!element) return;

    const label = document.querySelector<HTMLLabelElement>(
      'label[for="workspace-model"]'
    );
    element.hidden = true;
    label?.setAttribute("for", "workspace-model-picker");
    setSelect(element);

    const observer = new MutationObserver(() =>
      setRevision((current) => current + 1)
    );
    observer.observe(element, {
      attributes: true,
      childList: true,
      subtree: true
    });

    return () => {
      observer.disconnect();
      element.hidden = false;
      label?.setAttribute("for", "workspace-model");
    };
  }, []);

  if (!select?.parentElement) return null;

  const models = Array.from(select.options, (option) => ({
    id: option.value,
    label: option.text
  }));

  return createPortal(
    <ModelPicker
      disabled={select.disabled}
      key={revision}
      models={models}
      onValueChange={(value) => {
        select.value = value;
        select.dispatchEvent(new Event("change", { bubbles: true }));
      }}
      value={select.value}
    />,
    select.parentElement
  );
}

function ModelPicker({
  disabled,
  models,
  onValueChange,
  value
}: ModelPickerProps) {
  const listId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const selected = models.find((model) => model.id === value) ?? models[0];
  const [query, setQuery] = useState<string>(selected?.label ?? "");
  const [open, setOpen] = useState(false);
  const normalizedQuery = query.trim().toLowerCase();
  const filteredModels = models.filter(
    (model) =>
      normalizedQuery.length === 0 ||
      model.label.toLowerCase().includes(normalizedQuery) ||
      model.id.toLowerCase().includes(normalizedQuery)
  );

  function selectModel(model: ModelOption) {
    onValueChange(model.id);
    setQuery(model.label);
    setOpen(false);
    inputRef.current?.focus();
  }

  return (
    <div className="relative">
      <input
        aria-autocomplete="list"
        aria-controls={listId}
        aria-expanded={open}
        className="min-h-10 w-full rounded-md border border-input bg-background px-3 pr-9 text-sm focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
        disabled={disabled}
        id="workspace-model-picker"
        onBlur={(event) => {
          const list = document.getElementById(listId);
          if (event.relatedTarget && list?.contains(event.relatedTarget))
            return;
          setQuery(selected?.label ?? "");
          setOpen(false);
        }}
        onChange={(event) => {
          setQuery(event.target.value);
          setOpen(true);
        }}
        onFocus={() => {
          setQuery("");
          setOpen(true);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            setQuery(selected?.label ?? "");
            setOpen(false);
          } else if (event.key === "Enter" && open && filteredModels[0]) {
            event.preventDefault();
            selectModel(filteredModels[0]);
          } else if (event.key === "ArrowDown" && open && filteredModels[0]) {
            event.preventDefault();
            document.getElementById(`${listId}-0`)?.focus();
          }
        }}
        ref={inputRef}
        role="combobox"
        type="search"
        value={query}
      />
      <span
        aria-hidden="true"
        className="pointer-events-none absolute top-3 right-3 text-xs text-muted-foreground"
      >
        ▾
      </span>
      {open ? (
        <div
          className="absolute z-10 mt-1 max-h-64 w-full overflow-y-auto rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-lg"
          id={listId}
          role="listbox"
        >
          {filteredModels.length > 0 ? (
            filteredModels.map((model, index) => (
              <button
                aria-selected={model.id === value}
                className="flex w-full items-center justify-between gap-4 rounded-sm px-2.5 py-2 text-left text-sm hover:bg-accent focus:bg-accent focus:outline-none"
                id={`${listId}-${index}`}
                key={model.id}
                onBlur={(event) => {
                  if (
                    !event.currentTarget.parentElement?.contains(
                      event.relatedTarget
                    )
                  )
                    setOpen(false);
                }}
                onClick={() => selectModel(model)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    setQuery(selected?.label ?? "");
                    setOpen(false);
                    inputRef.current?.focus();
                  } else if (event.key === "ArrowDown") {
                    event.preventDefault();
                    document
                      .getElementById(
                        `${listId}-${Math.min(index + 1, filteredModels.length - 1)}`
                      )
                      ?.focus();
                  } else if (event.key === "ArrowUp") {
                    event.preventDefault();
                    if (index === 0) inputRef.current?.focus();
                    else
                      document
                        .getElementById(`${listId}-${index - 1}`)
                        ?.focus();
                  }
                }}
                onMouseDown={(event) => event.preventDefault()}
                role="option"
                type="button"
              >
                <span className="min-w-0">
                  <span className="block truncate font-medium">
                    {model.label}
                  </span>
                  <span className="block truncate text-xs text-muted-foreground">
                    {model.id}
                  </span>
                </span>
                {model.id === value ? (
                  <span aria-hidden="true" className="shrink-0">
                    ✓
                  </span>
                ) : null}
              </button>
            ))
          ) : (
            <p className="px-2.5 py-3 text-sm text-muted-foreground">
              No models found.
            </p>
          )}
        </div>
      ) : null}
    </div>
  );
}
