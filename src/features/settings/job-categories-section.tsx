import { ChevronDown, ChevronUp, Plus, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { SettingsRow, SettingsSection } from "~/components/settings-ui";
import { Button } from "~/components/ui/button";
import { Input, inputClassName } from "~/components/ui/input";
import { ApiError, api } from "~/lib/api";
import { cn } from "~/lib/utils";

/**
 * The job category rules in Settings › General (#199), administrators only: the ordered rules
 * (`GET /api/v1/settings/job-categories`), each with its category, field and pattern, moved,
 * added and deleted in place, saved as a whole list through `src/lib/api.ts` (a 422's
 * `ApiError.details` shown in the row it names), and a live preview of how many jobs each rule
 * would classify, refreshed 500 ms after the last edit.
 *
 * It renders inside the General settings form, so its buttons are `type="button"` (the default)
 * and it opens no `<form>` of its own.
 */

const RULES = "/api/v1/settings/job-categories";
const PREVIEW = "/api/v1/settings/job-categories/preview";
const PREVIEW_DEBOUNCE_MILLIS = 500;

/** The categories a rule may hold; `fix-agent` is never a rule's category. */
const CATEGORIES = ["test", "build", "check", "deploy", "other"] as const;
/** The fields of a job a rule's pattern is matched against. */
const FIELDS = ["job_name", "workflow_name", "repository", "labels"] as const;

type Rule = { category: string; field: string; pattern: string };
type RuleList = { rules: Array<{ position: number } & Rule> };
type Preview = {
  rules: Array<{ position: number; matches: number }>;
  other: Array<{ name: string; jobs: number }>;
};

const selectClassName = cn(inputClassName, "w-auto appearance-none pr-6");

export function JobCategoriesSection() {
  const [rules, setRules] = useState<Rule[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [rowErrors, setRowErrors] = useState<Record<number, string>>({});
  const [preview, setPreview] = useState<Preview | null>(null);
  const latestPreview = useRef(0);

  useEffect(() => {
    let cancelled = false;
    api<RuleList>(RULES)
      .then((list) => {
        if (cancelled) return;
        setRules(list.rules.map(({ category, field, pattern }) => ({ category, field, pattern })));
        setLoaded(true);
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setLoadError(error instanceof Error ? error.message : "The job category rules could not be read.");
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // The preview follows the rules as edited, half a second after the last change; a response
  // that arrives after a newer request is dropped.
  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      const request = latestPreview.current + 1;
      latestPreview.current = request;
      api<Preview>(PREVIEW, { body: { rules }, method: "POST" })
        .then((result) => {
          if (latestPreview.current === request) setPreview(result);
        })
        .catch(() => {
          /* a failed preview keeps the last one; saving reports its own errors */
        });
    }, PREVIEW_DEBOUNCE_MILLIS);
    return () => clearTimeout(timer);
  }, [loaded, rules]);

  const edit = (index: number, patch: Partial<Rule>) =>
    setRules((current) => current.map((rule, at) => (at === index ? { ...rule, ...patch } : rule)));

  const move = (index: number, offset: -1 | 1) =>
    setRules((current) => {
      const target = index + offset;
      const rule = current[index];
      const swap = current[target];
      if (rule === undefined || swap === undefined) return current;
      const next = [...current];
      next[index] = swap;
      next[target] = rule;
      return next;
    });

  const remove = (index: number) => setRules((current) => current.filter((_, at) => at !== index));

  const add = () => setRules((current) => [...current, { category: "test", field: "job_name", pattern: "" }]);

  async function save() {
    setSaving(true);
    setRowErrors({});
    try {
      const saved = await api<RuleList>(RULES, { body: { rules }, method: "PUT" });
      setRules(saved.rules.map(({ category, field, pattern }) => ({ category, field, pattern })));
      toast.success("Job categories saved.");
    } catch (error) {
      if (error instanceof ApiError && error.status === 422) {
        const byPosition: Record<number, string> = {};
        if (Array.isArray(error.details)) {
          for (const detail of error.details) {
            if (typeof detail !== "object" || detail === null) continue;
            const position = Number("position" in detail ? detail.position : Number.NaN);
            const message = "message" in detail ? detail.message : undefined;
            if (Number.isInteger(position) && typeof message === "string" && message) {
              byPosition[position] = message;
            }
          }
        }
        setRowErrors(byPosition);
      } else {
        toast.error(error instanceof Error ? error.message : "Could not save the job categories.");
      }
    } finally {
      setSaving(false);
    }
  }

  return (
    <SettingsSection
      description="How finished jobs are grouped on the fleet health page. The first rule that matches a job names its category; a job no rule matches is other."
      title="Job categories"
    >
      {loadError ? <p className="px-4 py-3 text-sm text-danger">{loadError}</p> : null}
      {!loaded && !loadError ? <p className="px-4 py-3 text-sm text-muted-foreground">Loading the rules…</p> : null}
      {rules.map((rule, index) => (
        <div className="flex flex-col gap-2 px-4 py-3" data-rule-index={index} key={index}>
          <div className="flex flex-wrap items-center gap-2">
            <select
              aria-label={`Category of rule ${index + 1}`}
              className={selectClassName}
              name="category"
              onChange={(event) => edit(index, { category: event.target.value })}
              value={rule.category}
            >
              {CATEGORIES.map((category) => (
                <option key={category} value={category}>{category}</option>
              ))}
            </select>
            <select
              aria-label={`Field matched by rule ${index + 1}`}
              className={selectClassName}
              name="field"
              onChange={(event) => edit(index, { field: event.target.value })}
              value={rule.field}
            >
              {FIELDS.map((field) => (
                <option key={field} value={field}>{field}</option>
              ))}
            </select>
            <Input
              aria-label={`Pattern of rule ${index + 1}`}
              className="min-w-40 flex-1 font-mono text-xs"
              name="pattern"
              onChange={(event) => edit(index, { pattern: event.target.value })}
              value={rule.pattern}
            />
            <span className="w-14 shrink-0 text-right text-xs tabular-nums text-muted-foreground" title="Jobs completed in the last 7 days this rule would classify">
              {preview?.rules[index]?.matches ?? "—"}
            </span>
            <Button aria-label="Move up" disabled={index === 0} onClick={() => move(index, -1)} size="icon-xs" variant="ghost"><ChevronUp /></Button>
            <Button aria-label="Move down" disabled={index === rules.length - 1} onClick={() => move(index, 1)} size="icon-xs" variant="ghost"><ChevronDown /></Button>
            <Button aria-label="Delete rule" onClick={() => remove(index)} size="icon-xs" variant="ghost"><Trash2 /></Button>
          </div>
          {rowErrors[index] ? <p className="text-xs text-danger">{rowErrors[index]}</p> : null}
        </div>
      ))}
      <SettingsRow
        description="Job names of the last 7 days no rule would match, most jobs first (the 20 commonest)."
        label="Most common in other"
        stacked
      >
        {preview === null ? (
          <p className="text-sm text-muted-foreground">Counting…</p>
        ) : preview.other.length > 0 ? (
          <ul className="flex flex-wrap gap-x-4 gap-y-1 text-sm text-muted-foreground">
            {preview.other.map((entry) => (
              <li key={entry.name}>
                <span className="font-medium text-foreground">{entry.name}</span> · {entry.jobs}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">No jobs of the last 7 days fall outside the rules.</p>
        )}
      </SettingsRow>
      <div className="flex items-center justify-between gap-3 px-4 py-3">
        <Button onClick={add} size="sm" variant="outline"><Plus />Add rule</Button>
        <Button disabled={saving || !loaded} onClick={() => void save()}>Save</Button>
      </div>
    </SettingsSection>
  );
}
