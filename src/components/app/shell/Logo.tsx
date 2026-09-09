import { useId } from "react";

import { cn } from "@/lib/cn";

/**
 * App mark — the bare M, no tile.
 *
 * The packaged icon (public/mnemos-mark.svg) carries a copper canvas because
 * the OS renders it against the user's wallpaper. In here we sit on our own
 * surface, so the tile would read as a sticker; the mark inverts with the
 * theme instead. Both colorways are the `--mark-*` stops in tokens.css:
 * bronze on light, cream on dark.
 *
 * Drawn at the mark's natural 1301×731, so callers set a height and let the
 * width follow.
 */
export function Logo({ className }: { className?: string }) {
  // Two Logos on one screen would otherwise share — and clobber — gradient ids.
  const id = useId();
  const frontL = `${id}-front-l`;
  const backL = `${id}-back-l`;
  const frontR = `${id}-front-r`;
  const backR = `${id}-back-r`;

  return (
    <svg
      aria-hidden="true"
      className={cn("w-auto shrink-0", className)}
      fill="none"
      viewBox="0 0 1301 731"
      xmlns="http://www.w3.org/2000/svg"
    >
      {/* userSpaceOnUse coordinates are read in the space the referencing line
			    sits in — i.e. before the <g> transform below — and each strand needs
			    its own gradient laid across its own width. */}
      <defs>
        {/* Across the stroke, not along it: the lift toward the centre is what
				    keeps each strand reading as a round tube rather than a flat bar. */}
        <linearGradient
          gradientUnits="userSpaceOnUse"
          id={frontL}
          x1="295.9"
          x2="484.1"
          y1="434.5"
          y2="540.5"
        >
          <stop offset="0" stopColor="var(--mark-front-1)" />
          <stop offset="0.32" stopColor="var(--mark-front-2)" />
          <stop offset="0.6" stopColor="var(--mark-front-3)" />
          <stop offset="1" stopColor="var(--mark-front-4)" />
        </linearGradient>
        {/* The back strands sit a half-step down so the crossing still reads as
				    an overlap once both strands share one hue. */}
        <linearGradient
          gradientUnits="userSpaceOnUse"
          id={backL}
          x1="585.9"
          x2="774.1"
          y1="540.5"
          y2="434.5"
        >
          <stop offset="0" stopColor="var(--mark-back-1)" />
          <stop offset="0.36" stopColor="var(--mark-back-2)" />
          <stop offset="0.68" stopColor="var(--mark-back-3)" />
          <stop offset="1" stopColor="var(--mark-back-4)" />
        </linearGradient>
        <linearGradient
          gradientUnits="userSpaceOnUse"
          id={frontR}
          x1="800.9"
          x2="989.1"
          y1="434.5"
          y2="540.5"
        >
          <stop offset="0" stopColor="var(--mark-front-1)" />
          <stop offset="0.32" stopColor="var(--mark-front-2)" />
          <stop offset="0.6" stopColor="var(--mark-front-3)" />
          <stop offset="1" stopColor="var(--mark-front-4)" />
        </linearGradient>
        <linearGradient
          gradientUnits="userSpaceOnUse"
          id={backR}
          x1="1090.9"
          x2="1279.1"
          y1="540.5"
          y2="434.5"
        >
          <stop offset="0" stopColor="var(--mark-back-1)" />
          <stop offset="0.36" stopColor="var(--mark-back-2)" />
          <stop offset="0.68" stopColor="var(--mark-back-3)" />
          <stop offset="1" stopColor="var(--mark-back-4)" />
        </linearGradient>
      </defs>

      <g strokeLinecap="round" strokeWidth="216" transform="translate(-137,-122)">
        <line stroke={`url(#${backL})`} x1="535" x2="825" y1="230" y2="745" />
        <line stroke={`url(#${frontL})`} x1="535" x2="245" y1="230" y2="745" />
        <line stroke={`url(#${backR})`} x1="1040" x2="1330" y1="230" y2="745" />
        <line stroke={`url(#${frontR})`} x1="1040" x2="750" y1="230" y2="745" />
      </g>
    </svg>
  );
}
