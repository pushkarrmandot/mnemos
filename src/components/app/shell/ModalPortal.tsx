/**
 * shadcn `Dialog` portal target (SHELL_CHEATSHEET.md §2, §5).
 *
 * W3 gives `useUIStore.modal` a single `ModalId`; this becomes a discriminated
 * switch over it. Single-slot by design — opening a second modal closes the
 * first, the app never stacks. W2 reserves the node.
 */
export function ModalPortal() {
  return <div data-mnemos-modal-root="" />;
}
