import { QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useChatStore } from "@/stores/chat";
import { useSelectionStore } from "@/stores/selection";
import { createTestQueryClient, mockRouteIPC } from "@/test/routeTestUtils";
import { ChatPane } from "./ChatPane";

/** A `chat_sessions` row as the backend would return it. */
function session(over: Record<string, unknown> = {}) {
  return {
    id: "chat-a",
    runner_id: "claude",
    scope_type: "project",
    scope_id: "p1",
    runner_session_id: null,
    title: "Battery decision",
    message_count: 4,
    total_input_tokens: 0,
    total_output_tokens: 0,
    cost_micros: 0,
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
    ...over,
  };
}

/** Records every command the pane actually invokes. */
let calls: string[] = [];

function mount(resolved: unknown, extra: Record<string, unknown> = {}) {
  calls = [];
  mockRouteIPC({
    chat_resolve_session: (payload: unknown) => {
      calls.push("chat_resolve_session");
      return typeof resolved === "function"
        ? (resolved as (p: unknown) => unknown)(payload)
        : resolved;
    },
    chat_list_sessions: () => {
      calls.push("chat_list_sessions");
      return [];
    },
    chat_get_session_history: () => {
      calls.push("chat_get_session_history");
      return [];
    },
    chat_send_prompt: () => {
      calls.push("chat_send_prompt");
      return { session_id: "chat-a" };
    },
    chat_start_new_session: () => {
      calls.push("chat_start_new_session");
      throw new Error("chat_start_new_session must not exist any more");
    },
    ...extra,
  });
  const client = createTestQueryClient();
  return render(
    <QueryClientProvider client={client}>
      <ChatPane onCollapse={vi.fn()} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  useSelectionStore.setState({ projectId: "p1", conversationId: null, chatSessionId: null });
  useChatStore.setState({ bySession: {}, outbox: [] });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("ChatPane", () => {
  it("keeps a chat that has messages when you navigate to another screen", async () => {
    // The regression this guards: you are mid-conversation in a project,
    // click something in the left nav, and the pane swaps to whatever the
    // new scope resolves to — your conversation vanishes. 06_CHAT.md: "If a
    // chat is already open, do NOT change scope."
    mount((payload: unknown) => {
      const scope = (payload as { scope?: { scope_type?: string } })?.scope;
      // Everything-scope has its own, different chat.
      return scope?.scope_type === "project"
        ? session()
        : session({ id: "chat-else", title: "Other" });
    });

    expect(await screen.findByText("Battery decision")).toBeInTheDocument();

    // Leaving the project route clears the project selection.
    act(() => useSelectionStore.getState().selectProject(null));

    await waitFor(() => {
      expect(screen.getByText("Battery decision")).toBeInTheDocument();
    });
    expect(screen.queryByText("Other")).not.toBeInTheDocument();
  });

  it("follows the scope when the pane is idle", async () => {
    // The other half: an untouched pane should track where you are, or the
    // chat would be pinned to wherever you first opened it forever.
    mount((payload: unknown) => {
      const scope = (payload as { scope?: { scope_type?: string } })?.scope;
      return scope?.scope_type === "project"
        ? session({ message_count: 0, title: "Empty project chat" })
        : session({ id: "chat-else", message_count: 0, title: "Everything chat" });
    });

    expect(await screen.findByText("Empty project chat")).toBeInTheDocument();

    act(() => useSelectionStore.getState().selectProject(null));

    expect(await screen.findByText("Everything chat")).toBeInTheDocument();
  });

  it("follows the scope on a fresh install, where no chat exists anywhere", async () => {
    // The empty-database case: every scope resolves to nothing, so the pane
    // opens a blank chat per scope. Navigating must retarget that blank
    // chat, or the composer would keep sending into the scope you started
    // in.
    mount(null, {
      list_projects: [{ id: "p1", name: "Acme", description: null, created_at: 1, updated_at: 1 }],
    });

    // Project scope: the chip names the project (not "Loading…", which is
    // what it showed while the picker's own lookups were gated on the
    // dropdown being open).
    await waitFor(() => {
      expect(screen.getByText("Project: Acme")).toBeInTheDocument();
    });

    act(() => useSelectionStore.getState().selectProject(null));

    await waitFor(() => {
      expect(screen.getByText("Everything")).toBeInTheDocument();
    });
    expect(screen.queryByText("Project: Acme")).not.toBeInTheDocument();
  });

  it("writes nothing to the backend when New chat is clicked repeatedly", async () => {
    // A row is created by the first *message*, never by opening a composer.
    // Clicking [+] used to mint a session row every time, which is how you
    // end up with a history full of empty chats.
    mount(session({ message_count: 0, title: "Idle chat" }));
    await screen.findByText("Idle chat");

    const before = calls.length;
    for (let i = 0; i < 3; i += 1) {
      fireEvent.click(screen.getByTitle("New chat"));
    }

    expect(calls.slice(before).filter((c) => c !== "chat_get_session_history")).toEqual([]);
  });

  it("lets a brand-new chat pick its scope, and locks it once the chat exists", async () => {
    // Scope is fixed *when the chat is created*, and a chat is not created
    // until its first message — so before that, choosing what it is about
    // is a normal thing to do, not a scope "change".
    // Everything-scope so the chip's label is stable without project data.
    useSelectionStore.setState({ projectId: null, conversationId: null });
    mount(session({ scope_type: "everything", scope_id: null, message_count: 0, title: "Fresh" }));
    await screen.findByText("Fresh");

    fireEvent.click(screen.getByTitle("New chat"));

    // Empty chat: the scope chip is a control, and opens the picker.
    const chip = await screen.findByRole("button", { name: /everything/i });
    fireEvent.click(chip);
    expect(await screen.findByText(/project/i)).toBeInTheDocument();
  });

  it("shows an existing chat's scope as a label, not a control", async () => {
    useSelectionStore.setState({ projectId: null, conversationId: null });
    mount(
      session({
        scope_type: "everything",
        scope_id: null,
        message_count: 4,
        title: "Has messages",
      }),
    );
    await screen.findByText("Has messages");

    // A chat with messages: the scope is shown, but is not a control.
    expect(screen.getByText("Everything")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /everything/i })).not.toBeInTheDocument();
  });

  it("gives each chat its own draft", async () => {
    // Drafts used to be keyed by *scope*, so two chats in one project shared
    // one composer buffer and typing in a new chat showed up in the old one.
    mount(session({ message_count: 0, title: "Draft chat" }));
    await screen.findByText("Draft chat"); // the pane knows which chat it is

    const composer = screen.getByPlaceholderText(/ask/i);
    fireEvent.change(composer, { target: { value: "first chat text" } });
    expect(composer).toHaveValue("first chat text");

    fireEvent.click(screen.getByTitle("New chat"));

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/ask/i)).toHaveValue("");
    });
  });
});
