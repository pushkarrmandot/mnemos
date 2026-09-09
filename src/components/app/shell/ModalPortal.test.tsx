import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { ModalPortal } from "@/components/app/shell/ModalPortal";
import { useUIStore } from "@/stores/ui";

/**
 * Modal row behavior: focus lands in the first input, Esc
 * closes, and an overlay click on a dirty form asks before discarding.
 */
/**
 * Radix arms its `pointerdown` listener on a timeout, and a primary-button
 * press is only treated as an outside interaction once the `click` lands.
 */
async function clickOutside() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  fireEvent.pointerDown(document.body);
  fireEvent.click(document.body);
}

function openNewProject() {
  useUIStore.setState({ modal: "new-project", modalProps: undefined });
}

describe("ModalPortal", () => {
  beforeEach(() => {
    useUIStore.setState({ modal: null, modalProps: undefined, toasts: [] });
  });

  it("renders nothing for an empty slot", () => {
    const { container } = render(<ModalPortal />);
    expect(container).toBeEmptyDOMElement();
  });

  it("puts focus in the name field, not the first button", async () => {
    openNewProject();
    render(<ModalPortal />);

    await waitFor(() => expect(screen.getByLabelText("Project name")).toHaveFocus());
  });

  it("closes on Esc", async () => {
    openNewProject();
    render(<ModalPortal />);

    fireEvent.keyDown(screen.getByLabelText("Project name"), { key: "Escape" });
    await waitFor(() => expect(useUIStore.getState().modal).toBeNull());
  });

  it("keeps a dirty form open and offers the discard step", async () => {
    openNewProject();
    render(<ModalPortal />);

    fireEvent.change(screen.getByLabelText("Project name"), { target: { value: "Q3" } });
    // Radix arms its outside-pointer listener a tick after the layer mounts.
    await clickOutside();

    await waitFor(() => expect(screen.getByText("Discard this project?")).toBeInTheDocument());
    expect(useUIStore.getState().modal).toBe("new-project");

    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(useUIStore.getState().modal).toBeNull());
  });

  it("lets a clean form close on an overlay click", async () => {
    openNewProject();
    render(<ModalPortal />);

    await clickOutside();
    await waitFor(() => expect(useUIStore.getState().modal).toBeNull());
  });

  it("creates the project and closes on Create", async () => {
    mockIPC((cmd) => {
      if (cmd !== "create_project") throw new Error(`unmocked command: ${cmd}`);
      return {
        id: "proj-1",
        name: "Q3 Redesign",
        description: null,
        pinned: false,
        archived: false,
        deleted_at: null,
        created_at: 0,
        updated_at: 0,
      };
    });
    openNewProject();
    render(<ModalPortal />);

    fireEvent.change(screen.getByLabelText("Project name"), { target: { value: "Q3 Redesign" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => expect(useUIStore.getState().modal).toBeNull());
    expect(useUIStore.getState().toasts).toHaveLength(0);
  });

  it("shows an error toast and keeps the modal open if creation fails", async () => {
    mockIPC(() => {
      throw { kind: "internal", message: "db locked" };
    });
    openNewProject();
    render(<ModalPortal />);

    fireEvent.change(screen.getByLabelText("Project name"), { target: { value: "Q3 Redesign" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => expect(useUIStore.getState().toasts).toHaveLength(1));
    expect(useUIStore.getState().modal).toBe("new-project");
  });
});
