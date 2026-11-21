/**
 * The connections list of Settings → Connections (#145): every platform's connections in one
 * section, driven by the platform descriptors. A descriptor's list binding is either page-derived
 * (rendered from the settings page data, nothing to load) or fetching (its own query, loading row,
 * error and Retry, so one platform's failure never hides another's). While every list has loaded
 * and all are empty, one generic empty state replaces the per-platform rows. Adding a connection
 * asks for the platform first, then shows that descriptor's fields or its link to connect elsewhere.
 * A value shown once (after a create, or returned by a row action) goes to the section's one
 * reveal dialog.
 */
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeftRight, ExternalLink, LoaderCircle, Plus } from "lucide-react";
import { type FormEvent, useState } from "react";
import { toast } from "sonner";

import { Callout, EmptyState, InlineError, LoadingRows } from "~/components/page";
import { SecretRevealDialog } from "~/components/secret-reveal-dialog";
import { SettingsSection } from "~/components/settings-ui";
import { StatusDot } from "~/components/status-icon";
import { Badge } from "~/components/ui/badge";
import { Button, buttonVariants } from "~/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "~/components/ui/dialog";
import { Input } from "~/components/ui/input";
import { SearchableSelect } from "~/components/ui/searchable-select";
import type { SettingsPage } from "~/features/operations/operations.functions";
import { useAction } from "~/lib/use-action";

import { platforms } from "./registry";
import type { ConnectionField, ConnectionList, ConnectionRowAction, PlatformDescriptor, SecretReveal } from "./types";

type FetchBinding = Extract<PlatformDescriptor["connections"]["list"], { kind: "fetch" }>;

/** Shows a value once in the section's reveal dialog. */
type OnReveal = (reveal: SecretReveal) => void;

/** Where one platform's list stands: a page-derived list is always ready. */
type ListState = { status: "loading" | "error" } | { status: "ready"; list: ConnectionList };

export const connectionsQueryKey = (platformId: string) => ["platform-connections", platformId] as const;

function connectionsQuery(descriptor: PlatformDescriptor, binding: FetchBinding) {
  return {
    queryKey: connectionsQueryKey(descriptor.id),
    queryFn: ({ signal }: { signal: AbortSignal }) => binding.load(signal),
  };
}

const fetchBinding = (descriptor: PlatformDescriptor): FetchBinding | null =>
  descriptor.connections.list.kind === "fetch" ? descriptor.connections.list : null;

export function ConnectionsSection({ descriptors = platforms, page }: { descriptors?: readonly PlatformDescriptor[]; page: SettingsPage }) {
  const queryClient = useQueryClient();
  const [reveal, setReveal] = useState<SecretReveal | null>(null);
  const [adding, setAdding] = useState(false);
  // Each opening starts the Add connection dialog afresh (no platform chosen, no values).
  const [addSession, setAddSession] = useState(0);
  const fetched = descriptors.flatMap((descriptor) => {
    const binding = fetchBinding(descriptor);
    return binding ? [{ descriptor, binding }] : [];
  });
  // Observes (never fetches) the lists the rows below load: their permission and whether they are empty.
  const observed = useQueries({ queries: fetched.map(({ descriptor, binding }) => ({ ...connectionsQuery(descriptor, binding), enabled: false })) });
  const states = descriptors.map((descriptor): ListState => {
    const index = fetched.findIndex((entry) => entry.descriptor === descriptor);
    if (index < 0) return { status: "ready", list: readList(descriptor, page) };
    const query = observed[index];
    if (query?.isError) return { status: "error" };
    return query?.data ? { status: "ready", list: query.data } : { status: "loading" };
  });
  const addable = descriptors.filter((descriptor, index) => {
    const state = states[index];
    const bindings = descriptor.connections;
    return Boolean(bindings.create || bindings.connectLink) && state?.status === "ready" && state.list.canManage;
  });
  // One generic empty state only once every list has loaded and none has a connection.
  const nothingConnected = states.every((state) => state.status === "ready" && state.list.items.length === 0);

  function openAdd() {
    setAddSession((session) => session + 1);
    setAdding(true);
  }

  return (
    <>
      <SettingsSection
        actions={addable.length && !nothingConnected ? <Button onClick={openAdd} size="sm" variant="outline"><Plus />Add connection</Button> : null}
        description="Every CI platform GridOps runs runners for. Runner pools pick their targets from these connections."
        title="Connections"
      >
        {nothingConnected ? (
          <EmptyState
            className="min-h-0 py-10"
            description={addable.length ? "Connect a CI platform to start assigning it to runner pools." : "A GridOps administrator can add platform connections."}
            icon={ArrowLeftRight}
            title="No connections yet"
          >
            {addable.length ? <Button onClick={openAdd}><Plus />Add connection</Button> : null}
          </EmptyState>
        ) : null}
        {/* Fetching platforms stay mounted under the generic empty state, so their queries stay active and reload after a create. */}
        {descriptors.map((descriptor) => {
          const binding = fetchBinding(descriptor);
          return binding
            ? <FetchedConnections binding={binding} descriptor={descriptor} key={descriptor.id} onReveal={setReveal} page={page} showEmpty={!nothingConnected} />
            : <ConnectionRows descriptor={descriptor} key={descriptor.id} list={readList(descriptor, page)} onReveal={setReveal} page={page} showEmpty={!nothingConnected} />;
        })}
      </SettingsSection>
      {addable.length ? (
        <AddConnectionDialog
          descriptors={addable}
          key={addSession}
          onCreated={async (descriptor, created) => {
            await queryClient.invalidateQueries({ queryKey: connectionsQueryKey(descriptor.id) });
            if (!descriptor.postCreate) return;
            // The dialog has closed: a failing hook reports through a toast, not dialog state.
            try {
              setReveal(await descriptor.postCreate(created));
            } catch (cause) {
              toast.error(cause instanceof Error ? cause.message : `${descriptor.name} was connected, but its setup did not finish.`);
            }
          }}
          onOpenChange={setAdding}
          open={adding}
          page={page}
        />
      ) : null}
      <SecretRevealDialog
        closeButtonText={reveal?.closeButtonText}
        description={reveal?.description ?? ""}
        details={reveal?.details}
        onClose={() => setReveal(null)}
        open={reveal !== null}
        secretLabel={reveal?.secretLabel ?? ""}
        title={reveal?.title ?? ""}
        value={reveal?.value ?? ""}
      />
    </>
  );
}

function readList(descriptor: PlatformDescriptor, page: SettingsPage): ConnectionList {
  const list = descriptor.connections.list;
  return list.kind === "page" ? list.read(page) : { canManage: false, items: [] };
}

/** One platform with a fetching list: its own query, loading row, error and Retry. */
function FetchedConnections({ descriptor, binding, page, showEmpty, onReveal }: { descriptor: PlatformDescriptor; binding: FetchBinding; page: SettingsPage; showEmpty: boolean; onReveal: OnReveal }) {
  const query = useQuery(connectionsQuery(descriptor, binding));
  if (query.isPending) return <LoadingRows rows={1} />;
  if (query.isError) {
    return (
      <div className="px-4 py-3">
        <InlineError className="mt-0" onRetry={() => void query.refetch()} title={`Couldn’t load ${descriptor.name} connections`}>
          {query.error instanceof Error ? query.error.message : undefined}
        </InlineError>
      </div>
    );
  }
  return <ConnectionRows descriptor={descriptor} list={query.data} onReveal={onReveal} page={page} showEmpty={showEmpty} />;
}

function ConnectionRows({ descriptor, list, page, showEmpty, onReveal }: { descriptor: PlatformDescriptor; list: ConnectionList; page: SettingsPage; showEmpty: boolean; onReveal: OnReveal }) {
  if (!list.items.length) {
    const text = showEmpty ? descriptor.connections.emptyText(page) : null;
    if (text === null) return null;
    return (
      <div className="flex items-center gap-3 px-4 py-4 text-sm text-muted-foreground">
        <PlatformBadge descriptor={descriptor} />
        <span>{text}</span>
      </div>
    );
  }
  return list.items.map((item) => <ConnectionRow canManage={list.canManage} descriptor={descriptor} item={item} key={descriptor.connections.describe(item).id} onReveal={onReveal} />);
}

export function PlatformBadge({ descriptor }: { descriptor: PlatformDescriptor }) {
  const Icon = descriptor.icon;
  return <Badge variant="outline"><Icon className="size-3" />{descriptor.name}</Badge>;
}

function ConnectionRow({ descriptor, item, canManage, onReveal }: { descriptor: PlatformDescriptor; item: unknown; canManage: boolean; onReveal: OnReveal }) {
  const queryClient = useQueryClient();
  const run = useAction();
  const bindings = descriptor.connections;
  const view = bindings.describe(item);
  const [editing, setEditing] = useState(false);
  const [values, setValues] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [showProblems, setShowProblems] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // While a row action or Remove runs, the row's actions are disabled.
  const [busy, setBusy] = useState(false);
  const editable = (bindings.fields ?? []).filter((field) => field.editableOnUpdate);
  const update = bindings.update;
  const remove = bindings.remove;
  const actions = bindings.actions?.(item) ?? [];
  const reload = () => queryClient.invalidateQueries({ queryKey: connectionsQueryKey(descriptor.id) });
  const showActions = canManage && !editing && Boolean(update || remove || actions.length);

  async function perform(options: Parameters<typeof run>[0]) {
    setBusy(true);
    try {
      await run(options);
    } finally {
      setBusy(false);
    }
  }

  async function runAction(action: ConnectionRowAction) {
    let reveal: SecretReveal | null = null;
    await perform({
      action: async () => { reveal = (await action.run(item)) ?? null; },
      after: reload,
      confirm: action.confirm,
      confirmOptions: action.confirmOptions,
      success: action.success ?? "Done.",
    });
    if (reveal) onReveal(reveal);
  }

  function startEditing() {
    setValues(bindings.editValues?.(item) ?? {});
    setError(null);
    setShowProblems(false);
    setEditing(true);
  }

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!update) return;
    if (fieldProblems(editable, values)) {
      setShowProblems(true);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await update(item, values);
      toast.success(bindings.updateSuccess?.(values) ?? "Connection updated.");
      setEditing(false);
      await reload();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "The connection could not be updated.");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div>
      {/* Below `sm` the status and actions move to their own line, so the name keeps the row's
          width; the badges wrap under a long name instead of squeezing it. */}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 px-4 py-3 sm:flex-nowrap">
        {view.leading ?? <StatusDot tone="success" />}
        <div className="min-w-0 flex-[1_1_calc(100%-3rem)] sm:flex-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="min-w-0 max-w-full truncate text-sm font-medium">{view.name}</span>
            <PlatformBadge descriptor={descriptor} />
            {view.badges}
          </div>
          {view.detail ? <div className="truncate text-xs text-muted-foreground">{view.detail}</div> : null}
        </div>
        {view.aside || showActions ? (
          <div className="flex w-full items-center justify-end gap-3 sm:w-auto sm:shrink-0">
            {view.aside}
            {/* The actions wrap, right-aligned, when a narrow row cannot hold them all on one line
                (a descriptor may contribute several); from `sm` up the row sizes this group to fit. */}
            {showActions ? (
              <div className="flex min-w-0 flex-wrap items-center justify-end gap-1">
                {update && editable.length ? <Button disabled={busy} onClick={startEditing} size="sm" variant="ghost">Edit</Button> : null}
                {actions.map((action) => (
                  <Button disabled={busy} key={action.label} onClick={() => void runAction(action)} size="sm" variant="ghost">{action.label}</Button>
                ))}
                {remove ? (
                  <Button
                    disabled={busy}
                    onClick={() => void perform({
                      action: () => remove(item),
                      after: reload,
                      confirm: bindings.removeConfirm?.(item) ?? `Remove ${view.name}?`,
                      success: bindings.removeSuccess?.(item) ?? `${view.name} removed.`,
                    })}
                    size="sm"
                    variant="ghost"
                  >
                    Remove
                  </Button>
                ) : null}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
      {editing ? (
        <form className="space-y-3 border-t border-border px-4 py-3" onSubmit={save}>
          <div className="grid gap-3 sm:grid-cols-2">
            {editable.map((field) => (
              <FieldInput
                field={field}
                id={`connection-edit-${descriptor.id}-${view.id}-${field.key}`}
                key={field.key}
                onChange={(value) => setValues((current) => ({ ...current, [field.key]: value }))}
                showProblem={showProblems}
                update
                value={values[field.key] ?? ""}
              />
            ))}
          </div>
          {error ? <Callout title="Could not save" tone="danger">{error}</Callout> : null}
          <div className="flex justify-end gap-2">
            <Button disabled={saving} onClick={() => setEditing(false)} type="button" variant="ghost">Cancel</Button>
            <Button disabled={saving} type="submit">{saving ? <LoaderCircle className="animate-spin" /> : null}{saving ? "Saving…" : "Save"}</Button>
          </div>
        </form>
      ) : null}
    </div>
  );
}

/** The fields' starting values on the create form: each field's `defaultValue`. */
function defaultValues(fields: readonly ConnectionField[] = []): Record<string, string> {
  return Object.fromEntries(fields.flatMap((field) => field.defaultValue === undefined ? [] : [[field.key, field.defaultValue]]));
}

/** Whether any field's own check rejects its value (a problem blocks submit). */
function fieldProblems(fields: readonly ConnectionField[], values: Record<string, string>) {
  return fields.some((field) => field.validate?.(values[field.key] ?? "") != null);
}

function FieldInput({ field, id, value, onChange, update = false, showProblem = false }: {
  field: ConnectionField;
  id: string;
  value: string;
  onChange: (value: string) => void;
  update?: boolean;
  /** Show a problem with an untouched (empty) value too, once a submit was refused. */
  showProblem?: boolean;
}) {
  const secret = field.type === "secret";
  const label = update ? field.updateLabel ?? field.label : field.label;
  const problem = value || showProblem ? field.validate?.(value) ?? null : null;
  const problemId = `${id}-problem`;
  return (
    <div>
      {field.type === "select" ? (
        <>
          <span className="mb-1.5 block text-xs font-medium text-foreground" id={`${id}-label`}>{label}</span>
          <SearchableSelect
            ariaLabel={label}
            onValueChange={(next) => onChange(next ?? "")}
            options={(field.options ?? []).map((option) => ({ value: option.value, label: option.label }))}
            searchable={false}
            value={value || null}
          />
        </>
      ) : (
        <>
          <label className="mb-1.5 block text-xs font-medium text-foreground" htmlFor={id}>{label}</label>
          <Input
            aria-describedby={problem ? problemId : undefined}
            aria-invalid={problem ? true : undefined}
            autoCapitalize="none"
            autoComplete="off"
            id={id}
            onChange={(event) => onChange(event.target.value)}
            placeholder={update ? field.updatePlaceholder ?? field.placeholder : field.placeholder}
            // On the edit form a blank secret keeps the stored one.
            required={field.required && !(update && secret)}
            type={secret ? "password" : "text"}
            value={value}
          />
        </>
      )}
      {problem ? <p className="mt-1.5 text-xs leading-5 text-danger" id={problemId}>{problem}</p> : null}
      {field.help ? <p className="mt-1.5 text-xs leading-5 text-muted-foreground">{field.help}</p> : null}
    </div>
  );
}

/**
 * Add connection: choose the platform first, then fill in only that platform's fields, or, for a
 * platform connected outside GridOps, follow its link. Lists only platforms the user can manage.
 */
function AddConnectionDialog({ descriptors, page, open, onOpenChange, onCreated }: {
  descriptors: readonly PlatformDescriptor[];
  page: SettingsPage;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (descriptor: PlatformDescriptor, created: unknown) => Promise<void>;
}) {
  const [chosen, setChosen] = useState<PlatformDescriptor | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [submitting, setSubmitting] = useState(false);
  const [showProblems, setShowProblems] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // A platform with a create binding gets its form; otherwise its connect link.
  const link = chosen && !chosen.connections.create ? chosen.connections.connectLink?.(page) ?? null : null;

  function choose(descriptor: PlatformDescriptor | null) {
    setChosen(descriptor);
    setValues(defaultValues(descriptor?.connections.fields));
    setShowProblems(false);
    setError(null);
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const create = chosen?.connections.create;
    if (!chosen || !create) return;
    // A field's own check blocks the request; nothing is sent until it passes.
    if (fieldProblems(chosen.connections.fields ?? [], values)) {
      setShowProblems(true);
      return;
    }
    setSubmitting(true);
    setError(null);
    let created: unknown;
    try {
      // A plain call, not a query mutation: a post-create value must never sit in a cache.
      created = await create(values);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : `${chosen.name} could not be connected.`);
      setSubmitting(false);
      return;
    }
    toast.success(chosen.connections.createSuccess ?? `${chosen.name} connection added.`);
    setSubmitting(false);
    onOpenChange(false);
    setValues({});
    // After the dialog closed: the reload and the post-create hook report their own errors.
    await onCreated(chosen, created);
  }

  const back = <Button disabled={submitting} onClick={() => choose(null)} type="button" variant="ghost">Back</Button>;

  return (
    <Dialog onOpenChange={(next) => { if (!submitting) onOpenChange(next); }} open={open}>
      <DialogContent aria-describedby={undefined} className="max-w-[520px]">
        <div className="space-y-4 overflow-y-auto p-5">
          <div>
            <DialogTitle className="text-base font-medium leading-6 text-foreground">
              {chosen ? chosen.connections.createTitle ?? `Connect ${chosen.name}` : "Add connection"}
            </DialogTitle>
            <DialogDescription className="mt-1.5 text-sm leading-5 text-muted-foreground">
              {chosen
                ? chosen.connections.create ? chosen.connections.createDescription ?? `Connect ${chosen.name} to GridOps.` : chosen.connections.chooserDescription
                : "Choose the platform to connect."}
            </DialogDescription>
          </div>
          {chosen && chosen.connections.create ? (
            <form className="space-y-3" onSubmit={submit}>
              {(chosen.connections.fields ?? []).map((field) => (
                <FieldInput
                  field={field}
                  id={`connection-new-${chosen.id}-${field.key}`}
                  key={field.key}
                  onChange={(value) => setValues((current) => ({ ...current, [field.key]: value }))}
                  showProblem={showProblems}
                  value={values[field.key] ?? ""}
                />
              ))}
              {error ? <Callout title="Could not connect" tone="danger">{error}</Callout> : null}
              <div className="flex justify-end gap-2">
                {back}
                <Button disabled={submitting} type="submit">
                  {submitting ? <LoaderCircle className="animate-spin" /> : <Plus />}
                  {submitting ? "Connecting…" : chosen.connections.createSubmitLabel ?? "Connect"}
                </Button>
              </div>
            </form>
          ) : chosen ? (
            <div className="space-y-3">
              {link ? <div className="text-sm leading-6 text-muted-foreground">{link.description}</div> : null}
              {link && link.href === null && link.unavailableText ? <Callout tone="warning">{link.unavailableText}</Callout> : null}
              <div className="flex justify-end gap-2">
                {back}
                {link?.href ? (
                  <a className={buttonVariants()} href={link.href} rel="noreferrer" target="_blank">{link.label}<ExternalLink /></a>
                ) : link ? (
                  <Button disabled type="button">{link.label}<ExternalLink /></Button>
                ) : null}
              </div>
            </div>
          ) : (
            <div className="grid gap-2">
              {descriptors.map((descriptor) => {
                const Icon = descriptor.icon;
                return (
                  <button
                    className="flex items-start gap-3 rounded-lg border border-border-strong px-3 py-2.5 text-left hover:bg-hover"
                    key={descriptor.id}
                    onClick={() => choose(descriptor)}
                    type="button"
                  >
                    <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                    <span className="min-w-0">
                      <span className="block text-sm font-medium text-foreground">{descriptor.name}</span>
                      <span className="mt-0.5 block text-xs leading-5 text-muted-foreground">{descriptor.connections.chooserDescription}</span>
                    </span>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
