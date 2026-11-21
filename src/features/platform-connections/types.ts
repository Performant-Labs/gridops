/**
 * The platform descriptor contract (#145). One descriptor per CI platform carries everything the
 * Connections page, the Targets row, the Platform options section and the runner row need, as
 * data and functions, so those surfaces hold no platform-specific code and a new platform is one
 * more descriptor module plus one line in `registry.tsx`.
 */
import type { ComponentType, ReactNode } from "react";
import type { LucideIcon } from "lucide-react";

import type { ConfirmOptions } from "~/components/confirm-dialog";

import type { Runner, SettingsPage } from "~/features/operations/operations.functions";
import type { RunnerPoolDetail, RunnerPoolOptions } from "~/features/runner-pools/runner-pools.functions";

// ---- Connections -------------------------------------------------------------------------------

/**
 * A value the Connections page shows once (e.g. a webhook secret), returned by a post-create hook
 * or by a row action. The page owns the one dialog that shows it.
 */
export type SecretReveal = {
  title: string;
  description: string;
  secretLabel: string;
  value: string;
  details?: ReactNode;
  closeButtonText?: string;
};

/** A platform's connections as one list binding resolves them. `items` are the platform's own records. */
export type ConnectionList = { canManage: boolean; items: unknown[] };

/**
 * How a platform's connections are listed. Declared by the descriptor, never inferred from its id:
 * - `fetch`: an async load that can fail; the Connections page gives it its own load state and Retry;
 * - `page`: read synchronously from the `/settings` page data; it cannot fail and has no load state.
 */
export type ConnectionListBinding =
  | { kind: "fetch"; load: (signal?: AbortSignal) => Promise<ConnectionList> }
  | { kind: "page"; read: (page: SettingsPage) => ConnectionList };

/** One field of a platform's connection form. */
export type ConnectionField = {
  key: string;
  label: string;
  /** `select` renders a choice among `options`, with `label` as its accessible name. */
  type: "text" | "secret" | "select";
  /** The choices of a `select` field. */
  options?: readonly { value: string; label: string }[];
  /** What the create form starts with. */
  defaultValue?: string;
  /** A client-side check of the value: the problem shown under the field, or null. A problem blocks submit. */
  validate?: (value: string) => string | null;
  required: boolean;
  help?: ReactNode;
  placeholder?: string;
  /** Shown on the edit form too. A secret left blank there is not sent, so the stored one is kept. */
  editableOnUpdate: boolean;
  /** Label and placeholder on the edit form, when they differ (e.g. "New API token"). */
  updateLabel?: string;
  updatePlaceholder?: string;
};

/** How one connection renders as a row on the Connections page. */
export type ConnectionRowView = {
  id: string;
  name: string;
  /** Replaces the default status dot (e.g. an avatar). */
  leading?: ReactNode;
  detail?: ReactNode;
  badges?: ReactNode;
  /** Platform-owned trailing content, e.g. an external "Configure" link. */
  aside?: ReactNode;
};

/**
 * One more action in a connection row's cluster, beside Edit and Remove. The row runs it through
 * `useAction` (the same option names), so it asks, reports and refreshes like Remove. A returned
 * reveal is shown once in the Connections page's dialog.
 */
export type ConnectionRowAction = {
  label: string;
  run: (item: unknown) => Promise<SecretReveal | null | void>;
  /** Asked first when set ("Question? Details."). */
  confirm?: string;
  confirmOptions?: ConfirmOptions;
  /** The success toast; defaults to "Done.". */
  success?: string;
};

export type ConnectionBindings = {
  list: ConnectionListBinding;
  /** Describes one item of `list` for display. */
  describe: (item: unknown) => ConnectionRowView;
  /**
   * The platform's empty row while its list is empty and some other platform has connections, or
   * null for no row (e.g. GitHub before its App exists). While every list is empty the Connections
   * page shows one generic empty state instead.
   */
  emptyText: (page: SettingsPage) => string | null;
  /** The one line under the platform's name in Add connection's platform chooser. */
  chooserDescription: string;
  /**
   * For a platform connected outside GridOps (e.g. by installing an app there): what Add connection
   * shows once the platform is chosen. `href: null` disables the link and shows `unavailableText`.
   */
  connectLink?: (page: SettingsPage) => { label: string; description: ReactNode; href: string | null; unavailableText?: string };
  fields?: readonly ConnectionField[];
  /** Create binding; its resolved value is what `postCreate` receives. */
  create?: (values: Record<string, string>) => Promise<unknown>;
  update?: (item: unknown, values: Record<string, string>) => Promise<unknown>;
  remove?: (item: unknown) => Promise<unknown>;
  /** Row actions shown, in order, between Edit and Remove for a manager. */
  actions?: (item: unknown) => readonly ConnectionRowAction[];
  /** Field values the edit form starts from. */
  editValues?: (item: unknown) => Record<string, string>;
  createTitle?: string;
  createDescription?: ReactNode;
  createSubmitLabel?: string;
  createSuccess?: string;
  updateSuccess?: (values: Record<string, string>) => string;
  removeConfirm?: (item: unknown) => string;
  removeSuccess?: (item: unknown) => string;
};

// ---- Targets -----------------------------------------------------------------------------------

/**
 * The already-loaded data the pool form was opened with, built once by the form and passed as the
 * same object to every kind's `load` / `available` / `offered` and to the payload partials. It holds
 * no load state: per-kind loading, error and not-connected states live only in the Targets row.
 */
export type TargetContext =
  | { form: "create"; options: RunnerPoolOptions }
  | { form: "update"; pool: RunnerPoolDetail };

/** One selectable target. `key` is `"<kindId>:<nativeId>"`, so ids from different kinds never collide. */
export type TargetOption = {
  key: string;
  kindId: string;
  /** The connection the target belongs to (installation, workspace, …). */
  connectionId: string;
  /** Unique within its kind across connections. */
  nativeId: string;
  label: string;
  description?: string;
  group: string;
  /** Position in the kind's loaded list; selections keep click order, this keeps list order. */
  loadIndex?: number;
  /** A placeholder whose `nativeId` comes from the kind's `manualEntry` input once picked. */
  manual?: boolean;
};

/** The selected targets per kind id, each kind in click order. A kind with nothing selected may be absent. */
export type ResolvedSelection = Readonly<Partial<Record<string, readonly TargetOption[]>>>;

/** Form values the per-kind rules depend on. */
export type TargetLimits = { maxCount: number };

/** A typed fallback for a target that cannot be listed (e.g. a runner group ID on a user account). */
export type ManualEntry = {
  /** Label of the input the Targets row renders for a picked manual option. */
  label: string;
  /** Turns the typed text into the option's `nativeId` for that connection, or null when invalid. */
  parse: (input: string, connectionId: string) => string | null;
  /** What the input starts with. */
  initialValue?: (context: TargetContext) => string;
};

export type TargetKind = {
  id: string;
  label: string;
  /** Group heading in the Targets picker: `<platform name> · <kind label>`. */
  group: string;
  cardinality: "many" | "one";
  /** Whether a pool may target only this kind. A non-standalone kind needs one of `requires`. */
  standalone: boolean;
  requires?: readonly string[];
  /** Shown on this kind's options while none of `requires` is selected. */
  requiresReason?: string;
  /** Kinds that cannot be selected together with this one. */
  excludes?: readonly string[];
  /** Shown on an excluded kind's options while this kind is selected. */
  exclusionReason?: string;
  /** Providers the pool must include once this kind is selected. */
  requiresProviders?: ReadonlyArray<"docker" | "tart">;
  /** Whether the platform has at least one connection in the already-loaded context. Synchronous. */
  available: (context: TargetContext) => boolean;
  /** Shown as the kind's group status when it is not available. */
  notConnected?: string;
  /** Loads the kind's options; owned by the kind so lazy lists need no picker change. */
  load: (context: TargetContext, signal: AbortSignal) => Promise<TargetOption[]>;
  /** Whether the form offers this kind for selection (default: always). Selected targets of a kind that is not offered show as locked chips. */
  offered?: (context: TargetContext) => boolean;
  /** Per-kind cap, counted on its own. */
  limit?: (limits: TargetLimits) => number;
  /** Selected targets of this kind raise the Maximum runner count's floor. */
  raisesMinMaxCount?: boolean;
  /** The pool form shows the Priority section for this kind. */
  supportsPriority?: boolean;
  manualEntry?: ManualEntry;
  /** The plan preview's target sentence for `count` selected targets of this kind. */
  planNote?: (count: number, limits: TargetLimits) => { ok: boolean; text: string };
  /** The kind whose `planNote` speaks while no kind with a plan note is selected. */
  defaultForPlan?: boolean;
};

// ---- Descriptor --------------------------------------------------------------------------------

/**
 * The pool form's Platform options values, per platform id: whatever each descriptor's
 * `platformOptions.initialValues` started and its `Component` reported back.
 */
export type PlatformOptionValues = Readonly<Record<string, unknown>>;

/** Form values the create partials need besides the selection (built at submit). */
export type TargetCreateValues = {
  platformOptions?: PlatformOptionValues;
};

/** What a platform's Platform options component is given: its own value and a way to replace it. */
export type PlatformOptionsProps = { values: unknown; onChange: (values: unknown) => void };

/** Form values the update partials need besides the selection (built at submit). */
export type TargetUpdateValues = {
  /** Selected target keys the operator prioritised. */
  priorityKeys: readonly string[];
  priorityWaitMinutes?: number;
  platformOptions?: PlatformOptionValues;
};

export type PayloadPartial = Record<string, unknown>;

/** What the new-pool page shows instead of the form while no standalone target kind is available. */
export type PrerequisiteEmptyState = {
  icon: LucideIcon;
  title: string;
  description: string;
  action?: { label: string; href: string };
};

export type PlatformDescriptor = {
  id: string;
  name: string;
  icon: ComponentType<{ className?: string }>;
  /** Classes for the platform badge's dot. */
  badgeDotClassName?: string;
  connections: ConnectionBindings;
  targetKinds: readonly TargetKind[];
  /** "Additional labels" help, placeholder and validator while the pool targets this platform. */
  labelRules?: { help?: ReactNode; placeholder?: string; validate?: (labels: string[]) => string | null };
  /** Content of the Platform options section while the pool targets this platform. */
  platformOptions?: {
    help: readonly ReactNode[];
    /** The platform's option values a form starts from: the pool's on settings, defaults on a new pool. */
    initialValues?: (pool?: RunnerPoolDetail) => unknown;
    Component?: ComponentType<PlatformOptionsProps>;
  };
  /** The runner's page on the platform, or null. */
  runnerLink?: (runner: Runner) => { href: string; label: string } | null;
  /** Runs after a connection is created; a returned value is shown once in a dialog. */
  // `created` is whatever this descriptor's `connections.create` resolves to.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  postCreate?: (created: any) => SecretReveal | null | Promise<SecretReveal | null>;
  /**
   * What the deliveries list shows as the destination of this platform's delivery (one whose id
   * starts with `<id>:`, or any unprefixed one for the fallback platform) when it has no
   * repository or account; none renders a dash.
   */
  deliveryDestinationFallback?: string;
  /**
   * Payload partials. Total: they never throw, and return the platform's empty/default partial
   * when it has no selected target. Validation stays in `schemas.ts` at submit time.
   */
  toCreatePayload: (selection: ResolvedSelection, context: TargetContext, values?: TargetCreateValues) => PayloadPartial;
  toUpdatePayload: (selection: ResolvedSelection, pool: RunnerPoolDetail, values: TargetUpdateValues) => PayloadPartial;
  /** The pool's current targets, for the settings form. */
  initialSelection?: (pool: RunnerPoolDetail) => ResolvedSelection;
  /** Keys of the pool's prioritised targets. */
  initialPriorityKeys?: (pool: RunnerPoolDetail) => string[];
  /** One line describing the pool's targets on this platform, or null when it has none. */
  readOnlySummary?: (pool: RunnerPoolDetail) => string | null;
  /** What the new-pool page shows while this platform's standalone kinds are unavailable. */
  prerequisiteEmptyState?: (context: TargetContext) => PrerequisiteEmptyState | null;
};
