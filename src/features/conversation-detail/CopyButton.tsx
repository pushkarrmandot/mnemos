import { Check, Copy } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/app/Button";

const RESET_MS = 1500;

/** Copies `text` to the clipboard and flashes a check mark for `RESET_MS`. */
export function CopyButton({
  text,
  label,
  className,
}: {
  text: string;
  label: string;
  /** W17b — lets per-row callers (`ExtractionLists`) add hover-reveal
   * classes without this component needing to know about that pattern. */
  className?: string;
}) {
  const [copied, setCopied] = useState(false);

  const handleClick = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), RESET_MS);
    } catch {
      // Clipboard access denied — nothing useful to surface for a
      // secondary affordance like this; the button just stays idle.
    }
  };

  return (
    <Button
      aria-label={copied ? "Copied" : label}
      className={className}
      onClick={handleClick}
      size="icon"
      variant="ghost"
    >
      {copied ? (
        <Check className="size-3.5 text-success" />
      ) : (
        <Copy className="size-3.5 text-tertiary" />
      )}
    </Button>
  );
}
