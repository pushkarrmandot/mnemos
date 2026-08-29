import { useQuery } from "@tanstack/react-query";
import { Check, ChevronDown, UserRound, X } from "lucide-react";
import { useState } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { qk } from "@/queries/keys";

/**
 * Who owes this — as an editable pill.
 *
 * The pill *is* the control. There is no separate edit affordance and no edit
 * mode, because the model gets attribution wrong often enough that correcting
 * it has to be cheaper than the mistake. An unassigned row shows a dashed
 * "Assign" in the same slot, so the gesture is in one place whether or not the
 * model got there first.
 *
 * Free text, no validation, no contact record required. There is no contacts
 * table until v1.3, and making someone create one before they can fix a name
 * would put a form in front of a one-word correction.
 *
 * Ordering of the menu is the design:
 *   1. **You**, always first — the most common correction is "that one's mine".
 *   2. Names heard in this meeting — the model's own speaker hints, which is a
 *      far better shortlist than an address book.
 *   3. Free text.
 *   4. Unassign, as its own row: clearing a wrong guess is a deliberate act,
 *      not an empty submit.
 */
export function AssigneePicker({
  onChange,
  suggestions,
  value,
}: {
  onChange: (next: string | null) => void;
  /** Names the model attributed elsewhere in this conversation. */
  suggestions: string[];
  value: string | null;
}) {
  const [draft, setDraft] = useState("");
  const [open, setOpen] = useState(false);

  const onboarding = useQuery({
    queryFn: () => commands.onboarding.getStatus(),
    queryKey: qk.onboardingStatus(),
  });
  const selfName = onboarding.data?.user_first_name?.trim() || null;

  const commit = (next: string | null) => {
    onChange(next);
    setDraft("");
    setOpen(false);
  };

  // "You" is stored literally, not as the user's name: it is what the model
  // emits for the mic channel, so keeping one representation means the two
  // paths cannot disagree about who the user is.
  const isSelf = value?.toLowerCase() === "you";
  const label = value ?? "Assign";

  return (
    <DropdownMenu onOpenChange={setOpen} open={open}>
      <DropdownMenuTrigger
        aria-label={value ? `Assigned to ${value}. Change assignee` : "Assign this action item"}
        className={cn(
          "motion-quick type-caption inline-flex items-center gap-1 rounded-full px-2 py-0.5",
          "transition-colors",
          value
            ? isSelf
              ? "bg-accent-primary-bg text-accent-primary-text hover:brightness-95"
              : "bg-subtle text-tertiary hover:bg-hover hover:text-secondary"
            : "border border-strong border-dashed text-tertiary hover:text-secondary",
        )}
      >
        {label}
        <ChevronDown aria-hidden="true" className="size-3 shrink-0" />
      </DropdownMenuTrigger>

      <DropdownMenuContent align="start" className="w-56">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            const trimmed = draft.trim();
            if (trimmed) commit(trimmed);
          }}
        >
          <input
            className={cn(
              "type-body mb-1 w-full rounded-sm border border-subtle bg-canvas px-2 py-1.5",
              "text-primary outline-none placeholder:text-tertiary focus:border-accent-primary",
            )}
            onChange={(e) => setDraft(e.target.value)}
            // Typing then Enter is the whole interaction for a name that is
            // not in either list, which is the common case in a big meeting.
            placeholder="Type a name…"
            value={draft}
          />
        </form>

        <DropdownMenuItem onSelect={() => commit("You")}>
          <UserRound aria-hidden="true" className="size-3.5 shrink-0" />
          <span className="flex-1">
            {selfName ? `${selfName} ` : ""}
            <span className="text-tertiary">(you)</span>
          </span>
          <Check aria-hidden="true" className={cn("size-3.5", !isSelf && "opacity-0")} />
        </DropdownMenuItem>

        {suggestions.length > 0 ? (
          <>
            <DropdownMenuSeparator />
            <p className="type-micro px-2 py-1 text-tertiary">Heard in this meeting</p>
            {suggestions.map((name) => (
              <DropdownMenuItem key={name} onSelect={() => commit(name)}>
                <span className="flex-1">{name}</span>
                <Check
                  aria-hidden="true"
                  className={cn("size-3.5", value !== name && "opacity-0")}
                />
              </DropdownMenuItem>
            ))}
          </>
        ) : null}

        {value ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => commit(null)}>
              <X aria-hidden="true" className="size-3.5 shrink-0" />
              Unassign
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * Names worth offering as shortcuts, drawn from what the model already
 * attributed elsewhere in the same conversation.
 *
 * `"Them"` is excluded for the same reason it is never rendered: it means "one
 * of the other people, unknown", so offering it as a choice would let someone
 * assign an item to nobody in particular and think they had assigned it.
 * `"You"` is excluded because it has its own pinned row above.
 */
export function assigneeSuggestions(hints: (string | null)[]): string[] {
  const seen = new Set<string>();
  for (const hint of hints) {
    const trimmed = hint?.trim();
    if (!trimmed) continue;
    const lower = trimmed.toLowerCase();
    if (lower === "them" || lower === "you") continue;
    seen.add(trimmed);
  }
  return [...seen].sort((a, b) => a.localeCompare(b));
}
