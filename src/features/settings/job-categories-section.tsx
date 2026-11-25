import { ChevronDown, ChevronUp, Plus, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
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
 * would classify, refreshed 500 ms after the last edit (a newer edit aborts the one in flight).
 *
 * #240: the page checks the server's limits itself (at most 200 rules, at most 1 KiB of UTF-8 per
 * pattern), names each problem where it is and sends nothing over them. Only a 422 pauses the
 * preview as an invalid rule, each message on its row; a server or network failure says the
 * preview is unavailable.
 *
 * It renders inside the General settings form, so its buttons are `type="button"` (the default)
 * and it opens no `<form>` of its own.
 */

const RULES = "/api/v1/settings/job-categories";
const PREVIEW = "/api/v1/settings/job-categories/preview";
const PREVIEW_DEBOUNCE_MILLIS = 500;
/** The server's limits (`parse_rules`, #240). */
const MAX_RULES = 200;
const MAX_PATTERN_BYTES = 1024;

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

const selectClassName = cn(inputClassName, "w-auto pr-7");

const utf8 = new TextEncoder();

/** What is over the server's limits: the list's length, and each pattern's size in UTF-8 bytes. */
function limitProblems(rules: Rule[]): { list: string | null; rows: Record<number, string> } {
  const rows: Record<number, string> = {};
  rules.forEach((rule, index) => {
    const bytes = utf8.encode(rule.pattern).length;
    if (bytes > MAX_PATTERN_BYTES) {
      rows[index] = `The pattern is ${bytes} bytes long; at most ${MAX_PATTERN_BYTES} (1 KiB).`;
    }
  });
  const list = rules.length > MAX_RULES ? `At most ${MAX_RULES} rules: the list has ${rules.length}.` : null;
  return { list, rows };
}

/** A 422's per-rule messages (`ApiError.details`, `[{position, message}]`), by position. */
function messagesByPosition(details: unknown): Record<number, string> {
  const byPosition: Record<number, string> = {};
  if (!Array.isArray(details)) return byPosition;
  for (const detail of details) {
    if (typeof detail !== "object" || detail === null) continue;
    const position = Number("position" in detail ? detail.position : Number.NaN);
    const message = "message" in detail ? detail.message : undefined;
    if (Number.isInteger(position) && typeof message === "string" && message) {
      byPosition[position] = message;
    }
  }
  return byPosition;
}

export function JobCategoriesSection() {
  const [rules, setRules] = useState<Rule[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [rowErrors, setRowErrors] = useState<Record<number, string>>({});
  const [preview, setPreview] = useState<Preview | null>(null);
  /** Why there is no preview: the 422's own message, or a server or network failure. */
  const [previewProblem, setPreviewProblem] = useState<{ invalid: boolean; message: string } | null>(null);
  const [previewRowErrors, setPreviewRowErrors] = useState<Record<number, string>>({});
  const limits = useMemo(() => limitProblems(rules), [rules]);
  const overLimits = limits.list !== null || Object.keys(limits.rows).length > 0;

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

  // The preview follows the rules as edited, half a second after the last change; a newer change
  // aborts the request in flight. Nothing is sent while the rules are over the limits.
  useEffect(() => {
    if (!loaded || overLimits) return;
    const controller = new AbortController();
    const timer = setTimeout(() => {
      api<Preview>(PREVIEW, { body: { rules }, method: "POST", signal: controller.signal })
        .then((result) => {
          if (controller.signal.aborted) return;
          setPreview(result);
          setPreviewProblem(null);
          setPreviewRowErrors({});
        })
        .catch((error: unknown) => {
          if (controller.signal.aborted) return;
          /* counts from a list the server refused would sit on the wrong rows
             after a move or delete, so they come off until it is accepted again */
          setPreview(null);
          if (error instanceof ApiError && error.status === 422) {
            const byPosition = messagesByPosition(error.details);
            setPreviewRowErrors(byPosition);
            setPreviewProblem({
              invalid: true,
              message: Object.keys(byPosition).length > 0
                ? "Preview paused until the rules marked above are fixed."
                : `Preview paused: ${error.message}`,
            });
          } else {
            setPreviewRowErrors({});
            setPreviewProblem({
              invalid: false,
              message: error instanceof ApiError
                ? `Preview unavailable: ${error.message}`
                : "Preview unavailable: GridOps could not be reached.",
            });
          }
        });
    }, PREVIEW_DEBOUNCE_MILLIS);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [loaded, overLimits, rules]);

  // Any change to the rules makes the last save's and the last preview's messages stale (a move
  // even shifts the position they name), so they come off with it until the next preview answers.
  const changeRules = (change: (current: Rule[]) => Rule[]) => {
    setRowErrors({});
    setPreviewRowErrors({});
    setPreviewProblem(null);
    setRules(change);
  };

  const edit = (index: number, patch: Partial<Rule>) =>
    changeRules((current) => current.map((rule, at) => (at === index ? { ...rule, ...patch } : rule)));

  const move = (index: number, offset: -1 | 1) =>
    changeRules((current) => {
      const target = index + offset;
      const rule = current[index];
      const swap = current[target];
      if (rule === undefined || swap === undefined) return current;
      const next = [...current];
      next[index] = swap;
      next[target] = rule;
      return next;
    });

  const remove = (index: number) => changeRules((current) => current.filter((_, at) => at !== index));

  const add = () =>
    changeRules((current) => (current.length >= MAX_RULES ? current : [...current, { category: "test", field: "job_name", pattern: "" }]));

  async function save() {
    if (overLimits) return;
    setSaving(true);
    setRowErrors({});
    try {
      const saved = await api<RuleList>(RULES, { body: { rules }, method: "PUT" });
      setRules(saved.rules.map(({ category, field, pattern }) => ({ category, field, pattern })));
      toast.success("Job categories saved.");
    } catch (error) {
      const byPosition = error instanceof ApiError && error.status === 422 ? messagesByPosition(error.details) : {};
      if (Object.keys(byPosition).length > 0) {
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
      {limits.list ? <p className="px-4 py-3 text-sm text-danger">{limits.list}</p> : null}
      {rules.map((rule, index) => {
        const rowError = rowErrors[index] ?? limits.rows[index] ?? previewRowErrors[index];
        return (
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
                {overLimits ? "—" : preview?.rules[index]?.matches ?? "—"}
              </span>
              {/* Each name carries the rule's number, as the fields' do, so a screen reader says which rule it acts on (#258). */}
              <Button aria-label={`Move rule ${index + 1} up`} disabled={index === 0} onClick={() => move(index, -1)} size="icon-xs" variant="ghost"><ChevronUp /></Button>
              <Button aria-label={`Move rule ${index + 1} down`} disabled={index === rules.length - 1} onClick={() => move(index, 1)} size="icon-xs" variant="ghost"><ChevronDown /></Button>
              <Button aria-label={`Delete rule ${index + 1}`} onClick={() => remove(index)} size="icon-xs" variant="ghost"><Trash2 /></Button>
            </div>
            {rowError ? <p className="text-xs text-danger">{rowError}</p> : null}
          </div>
        );
      })}
      <SettingsRow
        description="Job names of the last 7 days no rule would match, most jobs first (the 20 commonest)."
        label="Most common in other"
        stacked
      >
        {overLimits ? (
          <p className="text-sm text-muted-foreground">Preview paused until the rules are within the limits marked above.</p>
        ) : previewProblem ? (
          <p className={cn("text-sm", previewProblem.invalid ? "text-muted-foreground" : "text-danger")}>{previewProblem.message}</p>
        ) : preview === null ? (
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
        <Button disabled={rules.length >= MAX_RULES} onClick={add} size="sm" variant="outline"><Plus />Add rule</Button>
        <Button disabled={saving || !loaded || overLimits} onClick={() => void save()}>Save</Button>
      </div>
    </SettingsSection>
  );
}
