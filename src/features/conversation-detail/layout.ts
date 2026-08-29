/**
 * Shared reading-column width for long-form content (Summary markdown,
 * Transcript turns). Row-based sections (Action Items, Decisions, Open
 * Questions) intentionally stay full-width — they're short rows, not
 * paragraphs, and benefit from the extra horizontal space.
 *
 * Both `MarkdownView` and `TranscriptPane`/`LiveTranscriptPreview` must use
 * this exact constant — that's the whole point: one column width for every
 * "read a wall of text" surface in this view, so it can't drift out of sync
 * again like it did when `MarkdownView` had its own hardcoded `max-w-[68ch]`.
 */
export const READING_MAX_W = "max-w-[880px]";
