import { Fragment, type ReactNode } from "react";

/**
 * Compact markdown renderer for chat bubbles. Not a reuse of
 * `conversation-detail/markdown.tsx`'s `MarkdownView` — that one bakes in
 * `READING_MAX_W` and a full-page heading scale (`type-h3` for `#`) tuned
 * for a standalone document section, both wrong for a ~280px-wide bubble.
 * The parsing rules are intentionally identical (same three constructs:
 * heading/bullet/paragraph, same inline bold/italic) — only the sizing
 * differs — so if the source format ever grows real complexity, extract a
 * shared parser then; duplicating ~40 lines of straightforward line-scanning
 * twice is cheaper than a premature shared abstraction between two call
 * sites with different presentation needs.
 */
function renderInline(text: string, keyPrefix: string): ReactNode[] {
  const parts = text.split(/(\*\*[^*]+\*\*|\*[^*]+\*|_[^_]+_|`[^`]+`)/g).filter(Boolean);
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
    if (part.startsWith("`") && part.endsWith("`") && part.length > 1) {
      return (
        <code className="rounded bg-subtle px-1 py-0.5 font-mono text-[11.5px]" key={key}>
          {part.slice(1, -1)}
        </code>
      );
    }
    return <Fragment key={key}>{part}</Fragment>;
  });
}

export function ChatMarkdown({ text }: { text: string }) {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let listItems: string[] = [];
  let codeLines: string[] | null = null;
  let blockKey = 0;

  const flushList = () => {
    if (listItems.length === 0) return;
    const key = blockKey++;
    blocks.push(
      <ul className="my-1 list-disc space-y-0.5 pl-4" key={`ul-${key}`}>
        {listItems.map((item, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: static render of one streamed message, not a reorderable list
          <li key={i}>{renderInline(item, `li-${key}-${i}`)}</li>
        ))}
      </ul>,
    );
    listItems = [];
  };

  for (const rawLine of lines) {
    const line = rawLine.trimEnd();

    if (line.trim().startsWith("```")) {
      if (codeLines === null) {
        flushList();
        codeLines = [];
      } else {
        const key = blockKey++;
        blocks.push(
          <pre
            className="my-1.5 overflow-x-auto rounded-md border border-subtle bg-canvas p-2"
            key={`pre-${key}`}
          >
            <code className="font-mono text-[11.5px]">{codeLines.join("\n")}</code>
          </pre>,
        );
        codeLines = null;
      }
      continue;
    }
    if (codeLines !== null) {
      codeLines.push(rawLine);
      continue;
    }

    const heading = /^(#{1,3})\s+(.*)$/.exec(line);
    const bullet = /^[-*]\s+(.*)$/.exec(line);

    if (heading) {
      flushList();
      const text = heading[2] ?? "";
      const key = blockKey++;
      blocks.push(
        <h4
          className="type-caption mt-2.5 mb-0.5 font-semibold text-primary first:mt-0"
          key={`h-${key}`}
        >
          {renderInline(text, `h-${key}`)}
        </h4>,
      );
    } else if (bullet) {
      listItems.push(bullet[1] ?? "");
    } else if (line.trim() === "") {
      flushList();
    } else {
      flushList();
      const key = blockKey++;
      blocks.push(
        <p className="my-1 first:mt-0 last:mb-0" key={`p-${key}`}>
          {renderInline(line, `p-${key}`)}
        </p>,
      );
    }
  }
  flushList();

  return <div className="text-sm leading-[1.5]">{blocks}</div>;
}
