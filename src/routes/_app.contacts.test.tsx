import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Route } from "./_app.contacts";

describe("/_app/contacts route", () => {
  it("renders the coming-soon empty state without throwing", () => {
    const ContactsRoute = Route.options.component;
    if (!ContactsRoute) throw new Error("route has no component");
    render(<ContactsRoute />);

    expect(screen.getByText("Coming soon.")).toBeInTheDocument();
  });
});
