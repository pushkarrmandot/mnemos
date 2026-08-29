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
});
