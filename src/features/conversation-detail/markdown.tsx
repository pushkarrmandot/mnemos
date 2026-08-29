import { Fragment, type ReactNode } from "react";
import { READING_MAX_W } from "./layout";

/**
 * Minimal read-only markdown renderer for `summary.md`. No dependency added
 * for this — the shipping extraction prompt (LLD-05 §6.1) only ever produces
 * headings, bold/italic emphasis, and bullet/numbered lists, and summary
 * editing (tiptap, LLD-11 §10.2) isn't built this wave, so there is no
 * arbitrary user-authored markdown to round-trip yet. Revisit with a real
 * parser if/when editing lands.
 */
function renderInline(text: string, keyPrefix: string): ReactNode[] {
  const parts = text.split(/(\*\*[^*]+\*\*|\*[^*]+\*|_[^_]+_)/g).filter(Boolean);
  return parts.map((part, i) => {
    const key = `${keyPrefix}-${i}`;
    if (part.startsWith("**") && part.endsWith("**")) {
      return <strong key={key}>{part.slice(2, -2)}</strong>;
    }
    if (
      (part.startsWith("*") && part.endsWith("*")) ||
      (part.startsWith("_") && part.endsWith("_"))
    ) {
      return <em key={key}>{part.slice(1, -1)}</em>;
    }
    return <Fragment key={key}>{part}</Fragment>;
  });
}

export function MarkdownView({ markdown, className }: { markdown: string; className?: string }) {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let listItems: string[] = [];
  let blockKey = 0;

  const flushList = () => {
    if (listItems.length === 0) return;
    const key = blockKey++;
    blocks.push(
      <ul className="my-2 list-disc space-y-1 pl-5" key={`ul-${key}`}>
        {listItems.map((item, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: static render of a fixed markdown string, not a reorderable list
          <li key={i}>{renderInline(item, `li-${key}-${i}`)}</li>
        ))}
      </ul>,
    );
    listItems = [];
  };

  for (const rawLine of lines) {
    const line = rawLine.trimEnd();
    const heading = /^(#{1,3})\s+(.*)$/.exec(line);
    const bullet = /^[-*]\s+(.*)$/.exec(line);

    if (heading) {
      flushList();
      const level = (heading[1] ?? "#").length;
      const text = heading[2] ?? "";
      const Tag = level === 1 ? "h3" : level === 2 ? "h4" : "h5";
      // `type-h4`/`type-h5` don't exist (the named scale stops at
      // `type-h3`, DESIGN_SYSTEM.md §3) — this used to reference a class
      // that was never defined, so every `##`/`###` heading (what most real
      // extraction output actually uses — `## Discussion`/`## Decisions`/
      // etc., LLD-05's prompt) rendered with zero styling at all: no size,
      // no weight, indistinguishable from a paragraph. `#` stays `type-h3`
      // (unchanged, already correct) — the Section chrome above this content
      // ("Summary", with its icon) is itself `type-h3`, so a bigger in-body
      // heading here would visually outrank its own section title. `##`/`###`
      // differentiate by *weight*, not size, so they stay clearly
      // subordinate to both the section title and `#` while still reading
      // as unmistakably bolder than the `type-body` text beneath them.
      const sizeClass =
        level === 1 ? "type-h3" : level === 2 ? "type-body font-semibold" : "type-body font-medium";
      const key = blockKey++;
      blocks.push(
        <Tag className={`${sizeClass} mt-4 mb-1 text-primary first:mt-0`} key={`h-${key}`}>
          {renderInline(text, `h-${key}`)}
        </Tag>,
      );
    } else if (bullet) {
      listItems.push(bullet[1] ?? "");
    } else if (line.trim() === "") {
      flushList();
    } else {
      flushList();
      const key = blockKey++;
      blocks.push(
        <p className="my-2 text-primary first:mt-0" key={`p-${key}`}>
          {renderInline(line, `p-${key}`)}
        </p>,
      );
    }
  }
  flushList();

  return (
    <div className={`${READING_MAX_W} text-[16px] leading-[1.65] ${className ?? ""}`}>{blocks}</div>
  );
}
