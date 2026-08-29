import type { SearchScope } from "@/stores/cmdk";

/**
 * The Query key hierarchy (LLD-10 §4.2). Hierarchical readonly tuples —
 * invalidating a prefix invalidates every descendant, which is how the event
 * bridge (§6) invalidates whole subtrees with one call.
 *
 * Every key in the app is minted here. A feature that needs a new key adds it
 * to this object rather than inlining a tuple at the callsite.
 */
export const qk = {
  // Projects
  projects: () => ["projects"] as const,
  project: (id: string) => ["project", id] as const,
  projectMemory: (id: string) => ["project", id, "memory"] as const,
  /** Paged decision log for a project. */
  projectDecisions: (id: string) => ["project", id, "decisions"] as const,
  /** Paged action items for a project. `includeDone` is part of the key —
   * Open and Done are disjoint result sets, same reasoning as
   * `projectOpenQuestions`. */
  projectActionItems: (id: string, includeDone: boolean) =>
    ["project", id, "actionItems", includeDone] as const,
  /** Home's "Your to-dos", paged, `includeDone`-keyed the same way. */
  myActionItems: (includeDone: boolean) => ["actionItems", "mine", includeDone] as const,
  /** Home's "Project pulse" — one query, server-computed. */
  projectPulse: () => ["dashboard", "projectPulse"] as const,
  /** Paged open questions. Open and Resolved are disjoint result sets, so the
   * flag is part of the key rather than something the component filters. */
  projectOpenQuestions: (id: string, resolvedOnly: boolean) =>
    ["project", id, "openQuestions", resolvedOnly] as const,
  /** Is the synthesized memory behind, and did the last catch-up fail? */
  projectMemoryStatus: (id: string) => ["project", id, "memoryStatus"] as const,

  // Conversations
  /** Prefix for every conversation-list read. Invalidate this to refresh all
   * of them at once; never use it as a query key directly. */
  conversations: () => ["conversations"] as const,
  /**
   * One *scope* of conversations — the filter minus its paging fields (see
   * `conversationScopeKey`). The scope has to be in the key: Dashboard,
   * Recordings and the left nav all read conversations and all want different
   * subsets. They shared the bare `["conversations"]` key until W18, which was
   * harmless only because all three fetched byte-identical data; the moment
   * one of them started filtering, they would have been overwriting each
   * other's cache entry.
   */
  conversationsPage: (scope: object) => ["conversations", "page", scope] as const,
  /** `COUNT(*)` for a scope — the left nav's badges, which must not load rows
   * to render a number. */
  conversationsCount: (scope: object) => ["conversations", "count", scope] as const,
  conversation: (id: string) => ["conversation", id] as const,
  conversationTranscript: (id: string) => ["conversation", id, "transcript"] as const,
  conversationExtraction: (id: string) => ["conversation", id, "extraction"] as const,
  conversationPipeline: (id: string) => ["conversation", id, "pipeline"] as const,

  // Recovery
  /** Crash recovery scan (12_CORNER_CASES.md), run once on app boot. */
  interruptedRecordings: () => ["interruptedRecordings"] as const,
  /** Mid-processing crash recovery scan (12_CORNER_CASES.md), run once on app boot. */
  stuckProcessing: () => ["stuckProcessing"] as const,

  // Contacts
  contacts: () => ["contacts"] as const,
  contact: (id: string) => ["contact", id] as const,
  contactSearch: (q: string) => ["contact", "search", q] as const,

  // Chat
  chatSessions: () => ["chat", "sessions"] as const,
  chat: (sessionId: string) => ["chat", sessionId] as const,
  /** The backend session id resolved for a scope (`scopeKey`, chat's own
   * local key — see `chatScope.ts`), or `null` if that scope has never had
   * a session opened. */
  chatResolvedSession: (scopeKey: string) => ["chat", "resolved", scopeKey] as const,

  // Aggregates / dashboard
  actionItems: () => ["actionItems"] as const,
  actionItemsByConv: (cid: string) => ["actionItems", "byConv", cid] as const,
  calendarToday: () => ["calendar", "today"] as const,
  calendarUpcoming: () => ["calendar", "upcoming"] as const,

  // Search
  search: (scope: SearchScope, q: string) => ["search", scope, q] as const,

  // Onboarding
  onboardingStatus: () => ["onboarding", "status"] as const,

  // Settings & integrations
  settings: () => ["settings"] as const,
  integrations: () => ["integrations"] as const,
  mcpStatus: () => ["mcp", "status"] as const,
} as const;

/**
 * Per-key `staleTime` overrides (LLD-10 §4.3). Most reads are `Infinity` —
 * they change only through mutations and events, both of which invalidate
 * explicitly, so a timer-driven refetch is pure waste on a single-user desktop.
 */
export const staleTimes = {
  /** Changes only via mutations or events. */
  never: Number.POSITIVE_INFINITY,
  /** Actively rewritten by processing-progress fan-out. */
  live: 0,
  /** Autocomplete, debounced 200 ms upstream. */
  contactSearch: 10_000,
  /** ⌘K reopen must feel instant; content may drift. */
  search: 30_000,
  /** Matches the 60 s calendar poller. */
  calendar: 60_000,
} as const;
