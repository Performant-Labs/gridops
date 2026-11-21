import type { ReactNode } from "react";

import { CopyButton } from "~/components/copy-button";
import { Button } from "~/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "~/components/ui/dialog";

export type SecretRevealDialogProps = {
  open: boolean;
  /** The secret shown and copied. */
  value: string;
  title: string;
  description: ReactNode;
  /** The secret's `aria-label`. */
  secretLabel: string;
  /** The copy button's label; defaults to `secretLabel`. */
  copyLabel?: string;
  details?: ReactNode;
  closeButtonText?: string;
  onClose: () => void;
  /** Called once focus would return, after the dialog has closed. */
  onClosed?: () => void;
};

/**
 * Shows a secret once (an API token, a webhook secret). Escape and clicks outside are ignored so the
 * only way out is the explicit button; the body renders only while `open`, so the secret leaves the
 * DOM the moment the caller drops it from state. Callers keep the value out of any query cache.
 */
export function SecretRevealDialog({
  open,
  value,
  title,
  description,
  secretLabel,
  copyLabel,
  details,
  closeButtonText = "I’ve stored it, close",
  onClose,
  onClosed,
}: SecretRevealDialogProps) {
  return (
    <Dialog open={open}>
      <DialogContent
        // No exit animation: the secret leaves the DOM the moment it leaves state.
        className="top-[16vh] max-w-[520px]"
        instant
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          onClosed?.();
        }}
        onEscapeKeyDown={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
      >
        {open ? (
          <div className="space-y-4 p-5">
            <div>
              <DialogTitle className="text-base font-medium leading-6 text-foreground">{title}</DialogTitle>
              <DialogDescription className="mt-1.5 text-sm leading-5 text-muted-foreground">{description}</DialogDescription>
            </div>
            <div className="flex items-center gap-2 rounded-lg border border-border-strong bg-panel-subtle px-3 py-2">
              <code aria-label={secretLabel} className="min-w-0 flex-1 break-all font-mono text-xs text-foreground">{value}</code>
              <CopyButton label={copyLabel ?? secretLabel} size="sm" value={value} variant="outline">Copy</CopyButton>
            </div>
            {details}
            <div className="flex justify-end">
              <Button onClick={onClose}>{closeButtonText}</Button>
            </div>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
