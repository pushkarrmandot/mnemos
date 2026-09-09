import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Route } from "./_app.integrations";

describe("/_app/integrations route", () => {
  it("renders the coming-soon empty state without throwing", () => {
    const IntegrationsRoute = Route.options.component;
    if (!IntegrationsRoute) throw new Error("route has no component");
    render(<IntegrationsRoute />);

    expect(screen.getByText("Coming soon.")).toBeInTheDocument();
  });
});
