import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";

const container = document.getElementById("root");
if (!container) throw new Error("#root is missing from index.html");

// W2 wraps this in QueryClientProvider + ThemeProvider.
createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
