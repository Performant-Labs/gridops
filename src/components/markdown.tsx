import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { cn } from "~/lib/utils";

// Sized for dense panels: body copy at text-xs, headings barely larger.
// Raw HTML in the source is escaped, never rendered.
const components: Components = {
  h1: ({ node: _, ...props }) => <h3 className="mt-3 text-[13px] font-semibold text-foreground first:mt-0" {...props} />,
  h2: ({ node: _, ...props }) => <h3 className="mt-3 text-xs font-semibold text-foreground first:mt-0" {...props} />,
  h3: ({ node: _, ...props }) => <h4 className="mt-3 text-xs font-medium text-foreground first:mt-0" {...props} />,
  h4: ({ node: _, ...props }) => <h4 className="mt-3 text-xs font-medium text-foreground first:mt-0" {...props} />,
  p: ({ node: _, ...props }) => <p className="mt-1.5 first:mt-0" {...props} />,
  ul: ({ node: _, ...props }) => <ul className="mt-1.5 list-disc space-y-0.5 pl-4 first:mt-0" {...props} />,
  ol: ({ node: _, ...props }) => <ol className="mt-1.5 list-decimal space-y-0.5 pl-4 first:mt-0" {...props} />,
  blockquote: ({ node: _, ...props }) => <blockquote className="mt-1.5 border-l-2 border-border-strong pl-2.5 text-muted-foreground first:mt-0" {...props} />,
  a: ({ node: _, ...props }) => <a className="text-primary underline-offset-2 hover:underline" rel="noreferrer" target="_blank" {...props} />,
  strong: ({ node: _, ...props }) => <strong className="font-semibold text-foreground" {...props} />,
  hr: () => <hr className="my-3 border-border" />,
  pre: ({ node: _, ...props }) => <pre className="mt-1.5 overflow-x-auto rounded-md border border-border bg-background/60 p-2.5 font-mono text-2xs leading-4 first:mt-0 [&_code]:bg-transparent [&_code]:p-0" {...props} />,
  code: ({ node: _, ...props }) => <code className="break-words rounded bg-white/[0.06] px-1 py-px font-mono text-2xs text-foreground" {...props} />,
  table: ({ node: _, ...props }) => <div className="mt-1.5 overflow-x-auto first:mt-0"><table className="w-full border-collapse text-left" {...props} /></div>,
  th: ({ node: _, ...props }) => <th className="border-b border-border px-2 py-1 font-medium text-foreground" {...props} />,
  td: ({ node: _, ...props }) => <td className="border-b border-border/60 px-2 py-1 align-top" {...props} />,
};

export function Markdown({ children, className }: { children: string; className?: string }) {
  return (
    <div className={cn("break-words text-xs leading-5 text-secondary-foreground", className)}>
      <ReactMarkdown components={components} remarkPlugins={[remarkGfm]}>{children}</ReactMarkdown>
    </div>
  );
}
