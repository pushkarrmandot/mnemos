import { cn } from "@/lib/cn";

/**
 * DESIGN_SYSTEM.md §16 — illustration slots.
 *
 * v1 renders `<PlaceholderLine>`: one single-stroke figure per slot, drawn in
 * `--accent-primary` at 1.25px. v1.1 swaps in commissioned art behind the same
 * slot names. The contract that matters: **the slot always renders**, so the
 * layout doesn't jump when real art lands. Never feature-flag it away.
 */
export type IllustrationSlot =
  | "onboarding-hero"
  | "first-run-welcome"
  | "empty-dashboard"
  | "empty-project"
  | "empty-contacts"
  | "empty-search"
  | "sensitive-banner-icon"
  | "sadpath-generic"
  | "permission-mic"
  | "permission-screen";

/** 4:3 drawings share a 120×90 viewBox; 1:1 slots use 90×90. */
const SQUARE_SLOTS = new Set<IllustrationSlot>(["sensitive-banner-icon"]);

const paths: Record<IllustrationSlot, readonly string[]> = {
  // A horizon with the sun just over it — the start of something.
  "onboarding-hero": [
    "M14 66h92",
    "M40 66a20 20 0 0 1 40 0",
    "M60 28v8",
    "M34 40l6 6",
    "M86 40l-6 6",
  ],
  // An open doorway you're being let through.
  "first-run-welcome": ["M38 70V30a4 4 0 0 1 4-4h36a4 4 0 0 1 4 4v40", "M38 70h44", "M72 50h6"],
  // The memory thread, coiled — one continuous line turning inward.
  "empty-dashboard": [
    "M22 68c0-26 21-46 47-46s37 16 37 32-12 26-24 26-19-8-19-16 6-13 12-13 9 4 9 9",
  ],
  // Stacked conversations settling into one project.
  "empty-project": [
    "M30 40h60a4 4 0 0 1 4 4v28a4 4 0 0 1-4 4H30a4 4 0 0 1-4-4V44a4 4 0 0 1 4-4Z",
    "M34 30h52",
    "M42 20h36",
  ],
  // A person, reduced to the two arcs that read as one.
  "empty-contacts": ["M60 30a11 11 0 1 1 0 22 11 11 0 0 1 0-22Z", "M36 74a24 24 0 0 1 48 0"],
  // Magnifier, no results inside it.
  "empty-search": ["M54 24a22 22 0 1 1 0 44 22 22 0 0 1 0-44Z", "M70 62l18 18"],
  // A closed lock: this one is 1:1 and sits inline in a banner.
  "sensitive-banner-icon": [
    "M28 42h34a4 4 0 0 1 4 4v22a4 4 0 0 1-4 4H28a4 4 0 0 1-4-4V46a4 4 0 0 1 4-4Z",
    "M32 42V32a13 13 0 0 1 26 0v10",
  ],
  // A line that gets interrupted and picks back up.
  "sadpath-generic": ["M18 54h26", "M76 54h26", "M52 40l16 28", "M68 40l-16 28"],
  // Microphone capsule on a stand.
  "permission-mic": [
    "M60 22a9 9 0 0 1 9 9v18a9 9 0 0 1-18 0V31a9 9 0 0 1 9-9Z",
    "M42 46a18 18 0 0 0 36 0",
    "M60 64v10",
    "M48 74h24",
  ],
  // A screen being shared.
  "permission-screen": [
    "M24 26h72a4 4 0 0 1 4 4v34a4 4 0 0 1-4 4H24a4 4 0 0 1-4-4V30a4 4 0 0 1 4-4Z",
    "M50 68l-3 8",
    "M70 68l3 8",
    "M42 76h36",
  ],
};

type IllustrationProps = {
  slot: IllustrationSlot;
  /** `empty` caps at 240×180 (§16); `hero` at 480×360 for first-run surfaces. */
  scale?: "empty" | "hero" | "inline";
  className?: string;
};

const scaleClasses = {
  empty: "w-[240px] max-w-full",
  hero: "w-[480px] max-w-full",
  inline: "w-5",
} as const;

export function Illustration({ slot, scale = "empty", className }: IllustrationProps) {
  const square = SQUARE_SLOTS.has(slot);

  return (
    <svg
      aria-hidden="true"
      focusable="false"
      viewBox={square ? "0 0 90 90" : "0 0 120 90"}
      className={cn(scaleClasses[scale], square && "aspect-square", className)}
      fill="none"
      stroke="var(--accent-primary)"
      strokeWidth={1.25}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {paths[slot].map((d) => (
        <path key={d} d={d} />
      ))}
    </svg>
  );
}
