import { useEffect, useState } from "react";
import { type AppError, commands, describeError, type Pong } from "@/ipc";

type ProbeState =
  | { status: "pending" }
  | { status: "ok"; pong: Pong }
  | { status: "error"; error: AppError };

/**
 * W1 placeholder surface: it exists to prove the Rust ↔ React round-trip is
 * live. W2 replaces this entirely with the real `<AppShell>`.
 */
export default function App() {
  const [probe, setProbe] = useState<ProbeState>({ status: "pending" });

  useEffect(() => {
    let cancelled = false;

    commands
      .ping()
      .then((pong) => {
        if (!cancelled) setProbe({ status: "ok", pong });
      })
      .catch((error: AppError) => {
        if (!cancelled) setProbe({ status: "error", error });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main>
      <h1>Mnemos</h1>
      {probe.status === "pending" && <p>Connecting to the host…</p>}
      {probe.status === "ok" && (
        <p>
          Host v{probe.pong.app_version} · worker{" "}
          {probe.pong.worker_ready ? "ready" : "not started"}
        </p>
      )}
      {probe.status === "error" && <p role="alert">{describeError(probe.error)}</p>}
    </main>
  );
}
