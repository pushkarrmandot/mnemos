import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./design/global.css";
import { MeetingNotificationOverlay } from "@/features/meeting-notification/MeetingNotificationOverlay";

const container = document.getElementById("root");
if (!container) throw new Error("#root is missing from index.html");

// The meeting-detection overlay is a second, much smaller "app" sharing
// this one entry point rather than a route inside the main shell's router —
// see MeetingNotificationOverlay's own doc comment. Its window is created
// with a `?view=meeting-notification` URL by
// `commands::meeting_detection::maybe_show_overlay`.
const isMeetingNotification =
  new URLSearchParams(window.location.search).get("view") === "meeting-notification";

createRoot(container).render(
  <StrictMode>{isMeetingNotification ? <MeetingNotificationOverlay /> : <App />}</StrictMode>,
);
