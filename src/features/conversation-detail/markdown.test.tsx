import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MarkdownView } from "./markdown";

describe("MarkdownView", () => {
  it("renders headings, bold emphasis, and bullet lists", () => {
    render(
      <MarkdownView
        markdown={"# Summary\n\nWe agreed to **ship it**.\n\n- First point\n- Second point"}
      />,
    );

    expect(screen.getByRole("heading", { name: "Summary" })).toBeInTheDocument();
    expect(screen.getByText("ship it").tagName).toBe("STRONG");
    expect(screen.getByText("First point")).toBeInTheDocument();
    expect(screen.getByText("Second point")).toBeInTheDocument();
  });

  it("renders plain paragraphs when there is no markdown syntax", () => {
    render(<MarkdownView markdown={"Just a plain sentence."} />);
    expect(screen.getByText("Just a plain sentence.")).toBeInTheDocument();
  });
});
