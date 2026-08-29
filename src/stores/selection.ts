import { create } from "zustand";

/**
 * What the user currently has open (LLD-10 §3.5).
 *
 * Selection parameterizes Query keys — the chat pane reads
 * `useChatMessages(useSelectionStore(s => s.chatSessionId))`, so a selection
 * change fetches on its own. Routing writes selection on route change, never
 * the other way round: this store knows nothing about the router.
 */
type SelectionState = {
  projectId: string | null;
  conversationId: string | null;
  contactId: string | null;
  chatSessionId: string | null;

  selectProject: (id: string | null) => void;
  selectConversation: (id: string | null) => void;
  selectContact: (id: string | null) => void;
  selectChatSession: (id: string | null) => void;
};

export const useSelectionStore = create<SelectionState>()((set) => ({
  projectId: null,
  conversationId: null,
  contactId: null,
  chatSessionId: null,

  selectProject: (projectId) => set({ projectId }),
  selectConversation: (conversationId) => set({ conversationId }),
  selectContact: (contactId) => set({ contactId }),
  selectChatSession: (chatSessionId) => set({ chatSessionId }),
}));
