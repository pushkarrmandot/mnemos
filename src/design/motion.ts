/**
 * DESIGN_SYSTEM.md §14 — the motion vocabulary as JS values, for the cases CSS
 * can't reach (Framer Motion transitions, imperative animations). Curves are
 * never hand-typed in components; import from here.
 *
 * CSS consumers use the `motion-*` utilities from `design/global.css` instead.
 */
export type Ease = readonly [number, number, number, number];

export const motion = {
  quick: { duration: 0.12, ease: [0.4, 0, 0.2, 1] as Ease },
  tuck: { duration: 0.18, ease: [0.2, 0.8, 0.2, 1] as Ease },
  settle: { duration: 0.22, ease: [0.34, 1.56, 0.64, 1] as Ease },
  slide: { duration: 0.28, ease: [0.2, 0.8, 0.2, 1] as Ease },
  pulse: { duration: 1.4, ease: [0.4, 0, 0.6, 1] as Ease },
} as const;

/** Exits are always ≤ their enters; when in doubt, don't animate the exit. */
export const noExit = { duration: 0 } as const;
