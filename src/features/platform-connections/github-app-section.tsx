/**
 * The GitHub App section of Settings → Connections (#145), moved from the old GitHub settings page:
 * connection checks, endpoints, App creation (administrators, App not configured) and the
 * "Adding another GitHub account" note. Installations are not listed here: they are GitHub rows in
 * the connections list.
 */
import { ExternalLink, Github, LoaderCircle, Plus, RefreshCw } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { AsyncActionButton } from "~/components/async-action-button";
import { CopyButton } from "~/components/copy-button";
import { Callout } from "~/components/page";
import { SettingsRow, SettingsSection } from "~/components/settings-ui";
import { StatusDot } from "~/components/status-icon";
import { Button, buttonVariants } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { SearchableSelect } from "~/components/ui/searchable-select";
import { Tooltip } from "~/components/ui/tooltip";
import { type SettingsPage, createGitHubAppManifestAction, syncGitHubAction } from "~/features/operations/operations.functions";

type PageData = NonNullable<SettingsPage["data"]>;

export function GitHubAppSection({ configuration, githubApp, user }: {
  configuration: PageData["configuration"];
  githubApp: PageData["githubApp"];
  user: PageData["user"];
}) {
  const isAdmin = user.role === "admin";

  const checks = [
    { label: "GitHub OAuth", ready: configuration.githubOAuth, detail: "Client ID and secret" },
    { label: "Runner control", ready: configuration.githubAppControl, detail: "GitHub App installation credentials" },
    {
      label: "Webhook verification",
      ready: !configuration.webhookActive || configuration.webhookVerification,
      detail: configuration.webhookActive ? "HMAC signature secret" : "Delivery disabled; GridOps polls GitHub instead",
      status: configuration.webhookActive ? undefined : "Polling",
    },
    { label: "Encrypted storage", ready: configuration.secureStorage, detail: "Session and AES keys" },
    { label: "Manager authentication", ready: configuration.runnerManager, detail: "Internal bearer token" },
  ];

  return (
    <>
      <SettingsSection
        actions={githubApp ? (
          <>
            <AsyncActionButton action={() => syncGitHubAction()} icon={<RefreshCw />} success="GitHub installations and repository access refreshed.">Refresh access</AsyncActionButton>
            <a className={buttonVariants({ size: "sm", variant: "outline" })} href={githubApp.installUrl} rel="noreferrer" target="_blank"><Plus />Install</a>
            {isAdmin ? <a className={buttonVariants({ size: "sm", variant: "ghost" })} href={githubApp.appUrl} rel="noreferrer" target="_blank">Manage App<ExternalLink /></a> : null}
          </>
        ) : null}
        description={githubApp
          ? <>GridOps signs in with OAuth and operates runners through a private GitHub App, installed as <span className="font-mono text-foreground">{githubApp.slug}</span>. Repository pools can combine repositories from every installation you administer. Secrets are never shown here.</>
          : "GridOps signs in with OAuth and operates runners through a private GitHub App. Secrets are never shown here."}
        title="GitHub App"
      >
        {checks.map((check) => (
          <SettingsRow description={check.detail} key={check.label} label={check.label}>
            <span className="inline-flex items-center gap-2 text-sm text-secondary-foreground">
              <StatusDot tone={check.status ? "warning" : check.ready ? "success" : "danger"} />
              {check.status ?? (check.ready ? "Configured" : "Required")}
            </span>
          </SettingsRow>
        ))}
      </SettingsSection>

      <SettingsSection description="Configure these on the GitHub App." title="Endpoints">
        <CopyRow label="OAuth callback URL" value={configuration.callbackUrl} />
        <CopyRow label="Webhook URL" value={configuration.webhookUrl} />
      </SettingsSection>

      {isAdmin && !configuration.githubAppControl ? <CreateAppSection webhookActive={configuration.webhookActive} /> : null}

      {githubApp ? (
        <Callout title="Adding another GitHub account">
          GridOps-created Apps start private. In Manage App → Advanced, choose Make public once, then use Install. Organization owners approve installations requested by members.
        </Callout>
      ) : null}
    </>
  );
}

function CopyRow({ label, value }: { label: string; value: string }) {
  return (
    <SettingsRow label={label} stacked>
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded-md border border-border bg-panel-subtle px-2.5 py-1.5 font-mono text-xs text-secondary-foreground">{value}</code>
        <Tooltip content="Copy">
          <CopyButton label={label} size="icon-sm" value={value} variant="outline" />
        </Tooltip>
      </div>
    </SettingsRow>
  );
}

function CreateAppSection({ webhookActive }: { webhookActive: boolean }) {
  const [pending, setPending] = useState(false);
  const [ownerType, setOwnerType] = useState<"user" | "organization">("user");
  const [organization, setOrganization] = useState("");
  const [name, setName] = useState("GridOps Self-Hosted");

  async function createApp() {
    setPending(true);
    try {
      const setup = await createGitHubAppManifestAction({ data: {
        ownerType,
        organization: ownerType === "organization" ? organization.trim() : undefined,
        name: name.trim() || undefined,
      } });
      const form = document.createElement("form");
      form.action = `${setup.action}?state=${encodeURIComponent(setup.state)}`;
      form.method = "post";
      const manifest = document.createElement("input");
      manifest.type = "hidden";
      manifest.name = "manifest";
      manifest.value = setup.manifest;
      form.append(manifest);
      document.body.append(form);
      form.submit();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Could not start GitHub App setup.");
      setPending(false);
    }
  }

  return (
    <SettingsSection
      description="GridOps needs a private GitHub App to mint short-lived installation tokens with runner and Actions permissions. GitHub returns its credentials directly to this instance, where they are encrypted at rest."
      title="Create the GitHub App"
    >
      <SettingsRow label="Owner">
        <SearchableSelect
          ariaLabel="GitHub App owner"
          onValueChange={(next) => setOwnerType(next ?? "user")}
          options={[{ value: "user", label: "My account", description: "A user-owned GitHub App" }, { value: "organization", label: "An organization", description: "Owned by an organization" }]}
          searchable={false}
          value={ownerType}
        />
      </SettingsRow>
      {ownerType === "organization" ? (
        <SettingsRow htmlFor="app-org" label="Organization">
          <Input id="app-org" onChange={(event) => setOrganization(event.target.value)} placeholder="your-organization" required value={organization} />
        </SettingsRow>
      ) : null}
      <SettingsRow htmlFor="app-name" label="App name">
        <Input id="app-name" maxLength={100} onChange={(event) => setName(event.target.value)} value={name} />
      </SettingsRow>
      <div className="flex items-center justify-between gap-4 px-4 py-3">
        <p className="text-xs text-muted-foreground">{webhookActive ? "GitHub will deliver webhooks to this instance." : "Webhooks stay disabled for this private deployment; GridOps polls GitHub instead."}</p>
        <Button disabled={pending || (ownerType === "organization" && !organization.trim())} onClick={() => void createApp()}>
          {pending ? <LoaderCircle className="animate-spin" /> : <Github />}
          {pending ? "Opening GitHub…" : "Create GitHub App"}
        </Button>
      </div>
    </SettingsSection>
  );
}
