import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { RevealMore } from "./RevealMore";

describe("RevealMore", () => {
  it("renders nothing when there is no more to reveal — the floor rule", () => {
    // This is the property every paged list in the app depends on: a list
    // shorter than one page must render no control at all, so a new user
    // with four conversations sees exactly what they saw before pagination
    // existed anywhere in the app.
    const { container } = render(
      <RevealMore
        hasMore={false}
        isLoading={false}
        onClick={vi.fn()}
        pageSize={20}
        remaining={0}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("shows min(pageSize, remaining) as the count, not the raw page size", () => {
    render(
      <RevealMore hasMore={true} isLoading={false} onClick={vi.fn()} pageSize={20} remaining={5} />,
    );
    expect(screen.getByText(/show 5 more/i)).toBeInTheDocument();
  });

  it("caps the shown count at pageSize when more remains than one page", () => {
    render(
      <RevealMore
        hasMore={true}
        isLoading={false}
        onClick={vi.fn()}
        pageSize={20}
        remaining={108}
      />,
    );
    expect(screen.getByText(/show 20 more/i)).toBeInTheDocument();
    expect(screen.getByText("108 remaining")).toBeInTheDocument();
  });

  it("reads 'earlier' in the backward direction, not 'more'", () => {
    // Decisions read oldest-first, so their reveal sits above the list and
    // must not claim to be revealing "more" recent items — it reveals older
    // ones.
    render(
      <RevealMore
        direction="backward"
        hasMore={true}
        isLoading={false}
        onClick={vi.fn()}
        pageSize={20}
        remaining={34}
      />,
    );
    expect(screen.getByText(/show 20 earlier/i)).toBeInTheDocument();
    expect(screen.getByText("34 earlier")).toBeInTheDocument();
    expect(screen.queryByText(/more/i)).not.toBeInTheDocument();
  });

  it("disables the button and shows Loading… while fetching, without hiding the control", () => {
    render(
      <RevealMore hasMore={true} isLoading={true} onClick={vi.fn()} pageSize={20} remaining={5} />,
    );
    const button = screen.getByRole("button");
    expect(button).toBeDisabled();
    expect(button).toHaveTextContent(/loading/i);
  });

  /**
   * In the 208px left nav this rendered "Show 2 more" beside "2 remaining" —
   * the same number twice, wrapped across two ragged lines. The suffix is
   * only worth its width when it reports something past this page.
   */
  it("drops the remaining count when it just repeats the label's number", () => {
    render(
      <RevealMore hasMore={true} isLoading={false} onClick={vi.fn()} pageSize={10} remaining={2} />,
    );
    expect(screen.getByText(/show 2 more/i)).toBeInTheDocument();
    expect(screen.queryByText(/remaining/i)).toBeNull();
  });

  it("keeps the remaining count when more is left than this page reveals", () => {
    render(
      <RevealMore
        hasMore={true}
        isLoading={false}
        onClick={vi.fn()}
        pageSize={20}
        remaining={108}
      />,
    );
    expect(screen.getByText("108 remaining")).toBeInTheDocument();
  });

  /**
   * The suffix used to trail the label on the same row (`ml-auto`), which
   * only stayed readable above whatever width the two strings happened to
   * fit in — the 208px left nav was narrower than that. A fixed second row
   * is correct at any width, so this checks structure, not just that both
   * strings are present somewhere in the button.
   */
  it("puts the remaining count on its own line, not trailing the label", () => {
    render(
      <RevealMore
        hasMore={true}
        isLoading={false}
        onClick={vi.fn()}
        pageSize={20}
        remaining={108}
      />,
    );
    const label = screen.getByText("Show 20 more");
    const suffix = screen.getByText("108 remaining");
    expect(label.parentElement).not.toBe(suffix.parentElement);
  });
});
