import { Check, ChevronDown, Search, X } from "lucide-react";
import { type ReactNode, useEffect, useId, useMemo, useRef, useState } from "react";

import {
  type SearchableSelectOption,
  type SearchableSelectValue,
  filterSearchableOptions,
} from "~/components/ui/searchable-select";
import { cn } from "~/lib/utils";

/** Selected chips shown before the rest collapse into "+N more". */
const CHIP_LIMIT = 12;

/**
 * An option of the multi-select. The extra fields are optional, so plain
 * `SearchableSelectOption`s keep today's flat list:
 * - `group` puts the option under that group's heading;
 * - `disabled` blocks picking it (a selected option can still be removed), with `disabledReason` shown in the option;
 * - `locked` makes it a chip with no remove button that is not listed in the picker.
 */
export type SearchableMultiSelectOption<TValue extends SearchableSelectValue> = SearchableSelectOption<TValue> & {
  group?: string;
  disabled?: boolean;
  disabledReason?: string;
  icon?: ReactNode;
  locked?: boolean;
};

/** A heading in the picker, in display order, with an optional status (loading, error, not connected). */
export type SearchableMultiSelectGroup = { name: string; status?: ReactNode };

type SearchableMultiSelectProps<TValue extends SearchableSelectValue> = {
  options: Array<SearchableMultiSelectOption<TValue>>;
  values: TValue[];
  onValueChange: (values: TValue[]) => void;
  ariaLabel: string;
  placeholder?: string;
  searchPlaceholder?: string;
  emptyMessage?: string;
  loading?: boolean;
  disabled?: boolean;
  maxSelected?: number;
  selectedNoun?: string;
  /** Offer selecting or clearing every option that matches the search. */
  selectAll?: boolean;
  /** Group headings in order, each with an optional status. Options name their group. */
  groups?: SearchableMultiSelectGroup[];
};

export function SearchableMultiSelect<TValue extends SearchableSelectValue>({
  options,
  values,
  onValueChange,
  ariaLabel,
  placeholder = "Select options…",
  searchPlaceholder = "Search options…",
  emptyMessage = "No options found",
  loading = false,
  disabled = false,
  maxSelected,
  selectedNoun = "options",
  selectAll = false,
  groups,
}: SearchableMultiSelectProps<TValue>) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const [showAllChips, setShowAllChips] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listboxId = useId();
  const grouped = groups !== undefined || options.some((option) => option.group !== undefined);
  const groupNames = useMemo(() => {
    const names = (groups ?? []).map((group) => group.name);
    for (const option of options) if (option.group !== undefined && !names.includes(option.group)) names.push(option.group);
    return names;
  }, [groups, options]);
  // Grouped options are listed group by group; the active index counts across the whole list.
  const filteredOptions = useMemo(() => {
    // The filter returns a subset of the options it is given, so their extra fields are kept.
    const listed = filterSearchableOptions(options.filter((option) => !option.locked), query) as Array<SearchableMultiSelectOption<TValue>>;
    if (!grouped) return listed;
    return [
      ...listed.filter((option) => option.group === undefined),
      ...groupNames.flatMap((name) => listed.filter((option) => option.group === name)),
    ];
  }, [groupNames, grouped, options, query]);
  const selectedOptions = values
    .map((value) => options.find((option) => option.value === value))
    .filter((option): option is SearchableMultiSelectOption<TValue> => option !== undefined);

  useEffect(() => {
    function closeOnOutsidePointer(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node)) close();
    }
    document.addEventListener("pointerdown", closeOnOutsidePointer);
    return () => document.removeEventListener("pointerdown", closeOnOutsidePointer);
  }, []);

  useEffect(() => {
    if (open) window.requestAnimationFrame(() => searchInputRef.current?.focus());
  }, [open]);

  function close({ restoreFocus = false } = {}) {
    setOpen(false);
    setQuery("");
    if (restoreFocus) window.requestAnimationFrame(() => triggerRef.current?.focus());
  }

  function toggle(value: TValue) {
    if (values.includes(value)) {
      if (options.find((option) => option.value === value)?.locked) return;
      onValueChange(values.filter((selected) => selected !== value));
      return;
    }
    if (options.find((option) => option.value === value)?.disabled) return;
    if (maxSelected !== undefined && values.length >= maxSelected) return;
    onValueChange([...values, value]);
  }

  const allFilteredSelected =
    filteredOptions.length > 0 && filteredOptions.every((option) => values.includes(option.value));

  function toggleAllFiltered() {
    const filtered = new Set(filteredOptions.map((option) => option.value));
    if (allFilteredSelected) {
      onValueChange(values.filter((value) => !filtered.has(value)));
      return;
    }
    const added = filteredOptions
      .filter((option) => !option.disabled)
      .map((option) => option.value)
      .filter((value) => !values.includes(value));
    const room = maxSelected === undefined ? added.length : Math.max(0, maxSelected - values.length);
    onValueChange([...values, ...added.slice(0, room)]);
  }

  function moveActive(direction: 1 | -1) {
    if (filteredOptions.length === 0) return;
    setActiveIndex((current) => (current + direction + filteredOptions.length) % filteredOptions.length);
  }

  function toggleActive() {
    const option = filteredOptions[activeIndex];
    if (option) toggle(option.value);
  }

  function renderOption(option: SearchableMultiSelectOption<TValue>) {
    const index = filteredOptions.indexOf(option);
    const selected = values.includes(option.value);
    const atLimit = !selected && maxSelected !== undefined && values.length >= maxSelected;
    const blocked = atLimit || (!selected && option.disabled === true);
    return (
      <button
        aria-selected={selected}
        className={cn(
          "flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-popover-hover",
          index === activeIndex && "bg-popover-hover",
          blocked && "cursor-not-allowed opacity-40",
        )}
        disabled={blocked}
        id={`${listboxId}-option-${index}`}
        key={String(option.value)}
        onClick={() => toggle(option.value)}
        onMouseEnter={() => setActiveIndex(index)}
        role="option"
        tabIndex={-1}
        type="button"
      >
        <span className={cn(
          "grid size-4 shrink-0 place-items-center rounded border",
          selected ? "border-primary bg-primary text-primary-foreground" : "border-border-strong",
        )}>
          {selected ? <Check className="size-3" /> : null}
        </span>
        {option.icon ? <span className="shrink-0 text-muted-foreground">{option.icon}</span> : null}
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm text-foreground">{option.label}</span>
          {option.description ? <span className="mt-0.5 block truncate text-2xs text-muted-foreground">{option.description}</span> : null}
          {!selected && option.disabled && option.disabledReason ? <span className="mt-0.5 block truncate text-2xs text-muted-foreground">{option.disabledReason}</span> : null}
        </span>
      </button>
    );
  }

  const summary = selectedOptions.length === 0
    ? placeholder
    : selectedOptions.length === 1
      ? selectedOptions[0]?.label
      : `${selectedOptions.length} ${selectedNoun} selected`;

  return (
    <div ref={rootRef}>
      <div className="relative">
      <button
        aria-controls={listboxId}
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-label={ariaLabel}
        className={cn(
          "flex h-8 w-full items-center justify-between gap-2 rounded-md border border-border-strong bg-panel px-2.5 text-left text-sm text-foreground shadow-[0_1px_1px_rgb(0_0_0/0.03)] outline-none transition-colors",
          "hover:bg-hover focus-visible:border-primary/70 focus-visible:ring-2 focus-visible:ring-primary/20",
          open && "border-primary/70 ring-2 ring-primary/20",
          (disabled || loading) && "cursor-not-allowed opacity-50",
        )}
        disabled={disabled || loading}
        onClick={() => setOpen((current) => {
          const next = !current;
          if (next) setActiveIndex(0);
          return next;
        })}
        ref={triggerRef}
        type="button"
      >
        <span className={cn("min-w-0 flex-1 truncate", values.length === 0 && "text-muted-foreground")}>
          {loading ? `Loading ${selectedNoun}…` : summary}
        </span>
        <ChevronDown className={cn("size-4 shrink-0 text-muted-foreground transition-transform", open && "rotate-180")} />
      </button>

      {open ? (
        <div className="absolute left-0 top-full z-50 mt-1 w-full min-w-72 overflow-hidden rounded-lg border border-border-strong bg-popover text-left shadow-popover">
          <div className="border-b border-border p-1">
            <div className="relative">
              <Search className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
              <input
                aria-activedescendant={filteredOptions[activeIndex] ? `${listboxId}-option-${activeIndex}` : undefined}
                aria-controls={listboxId}
                aria-expanded={open}
                aria-label={`Search ${ariaLabel}`}
                className="h-8 w-full rounded-md bg-transparent pl-8 pr-3 text-sm outline-none placeholder:text-faint"
                onChange={(event) => {
                  setQuery(event.target.value);
                  setActiveIndex(0);
                }}
                onKeyDown={(event) => {
                  if (event.key === "ArrowDown") {
                    event.preventDefault();
                    moveActive(1);
                  } else if (event.key === "ArrowUp") {
                    event.preventDefault();
                    moveActive(-1);
                  } else if (event.key === "Home") {
                    event.preventDefault();
                    setActiveIndex(0);
                  } else if (event.key === "End") {
                    event.preventDefault();
                    setActiveIndex(Math.max(0, filteredOptions.length - 1));
                  } else if (event.key === "Enter") {
                    event.preventDefault();
                    toggleActive();
                  } else if (event.key === "Escape") {
                    event.preventDefault();
                    close({ restoreFocus: true });
                  }
                }}
                placeholder={searchPlaceholder}
                ref={searchInputRef}
                role="combobox"
                value={query}
              />
            </div>
            {maxSelected !== undefined || selectAll ? (
              <div className="mt-1 flex items-center justify-between gap-2 px-1 pb-0.5">
                <span className="text-2xs text-muted-foreground">
                  {maxSelected !== undefined ? `${values.length} of ${maxSelected} ${selectedNoun} selected` : `${values.length} selected`}
                </span>
                {selectAll && filteredOptions.length > 0 ? (
                  <button
                    className="rounded px-1.5 py-0.5 text-2xs font-medium text-primary hover:bg-popover-hover"
                    onClick={toggleAllFiltered}
                    type="button"
                  >
                    {allFilteredSelected
                      ? query ? "Clear matches" : "Clear all"
                      : query ? `Select ${filteredOptions.length} matches` : "Select all"}
                  </button>
                ) : null}
              </div>
            ) : null}
          </div>
          <div aria-label={ariaLabel} className="max-h-72 overflow-y-auto p-1" id={listboxId} role="listbox" aria-multiselectable="true">
            {filteredOptions.length === 0 && !(groups ?? []).some((group) => group.status) ? (
              <div className="px-3 py-6 text-center text-xs text-muted-foreground">{emptyMessage}</div>
            ) : grouped ? (
              <>
                {filteredOptions.filter((option) => option.group === undefined).map((option) => renderOption(option))}
                {groupNames.map((name) => {
                  const members = filteredOptions.filter((option) => option.group === name);
                  const status = groups?.find((group) => group.name === name)?.status;
                  if (!members.length && !status) return null;
                  return (
                    <div key={name} role="group" aria-label={name}>
                      <div className="px-2 pb-1 pt-2 text-2xs font-medium uppercase tracking-wide text-faint" role="presentation">{name}</div>
                      {members.map((option) => renderOption(option))}
                      {status ? <div className="px-2 pb-1.5 text-xs text-muted-foreground">{status}</div> : null}
                    </div>
                  );
                })}
              </>
            ) : filteredOptions.map((option) => renderOption(option))}
          </div>
        </div>
      ) : null}
      </div>
      {selectedOptions.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {(showAllChips ? selectedOptions : selectedOptions.slice(0, CHIP_LIMIT)).map((option) => option.locked ? (
            <span
              className="inline-flex h-6 max-w-full items-center gap-1 rounded-full border border-border bg-panel-subtle px-2 text-2xs font-medium text-muted-foreground"
              key={String(option.value)}
              title="Kept as it is"
            >
              {option.icon ? <span className="shrink-0">{option.icon}</span> : null}
              <span className="max-w-56 truncate">{option.label}</span>
            </span>
          ) : (
            <button
              aria-label={`Remove ${option.label}`}
              className="inline-flex h-6 max-w-full items-center gap-1 rounded-full border border-border-strong bg-panel-subtle px-2 text-2xs font-medium text-secondary-foreground hover:border-faint hover:text-foreground"
              disabled={disabled}
              key={option.value}
              onClick={() => toggle(option.value)}
              type="button"
            >
              <span className="max-w-56 truncate">{option.label}</span>
              <X className="size-3 text-muted-foreground" />
            </button>
          ))}
          {selectedOptions.length > CHIP_LIMIT ? (
            <button
              className="inline-flex h-6 items-center rounded-full px-2 text-2xs font-medium text-primary hover:bg-hover"
              onClick={() => setShowAllChips((current) => !current)}
              type="button"
            >
              {showAllChips ? "Show less" : `+${selectedOptions.length - CHIP_LIMIT} more`}
            </button>
          ) : null}
        </div>
      ) : null}

    </div>
  );
}
