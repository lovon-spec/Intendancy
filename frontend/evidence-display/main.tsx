import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../src/index.css";
import "./display.css";
import { App } from "./App";

// Surface script failures on the page itself: a juror on an unusual browser
// should see the error, not a blank frame.
function showFailure(message: string) {
  const root = document.getElementById("root");
  if (!root) return;
  const box = document.createElement("pre");
  box.style.cssText = "margin:16px;padding:12px;border:1px solid #fca5a5;background:#fef2f2;color:#7f1d1d;font:12px ui-monospace,Menlo,monospace;white-space:pre-wrap";
  box.textContent = `The evidence display hit an error in this browser:\n${message}`;
  root.prepend(box);
}
window.addEventListener("error", (event) => showFailure(event.error instanceof Error ? `${event.error.message}\n${event.error.stack ?? ""}` : String(event.message)));
window.addEventListener("unhandledrejection", (event) => showFailure(event.reason instanceof Error ? `${event.reason.message}\n${event.reason.stack ?? ""}` : String(event.reason)));

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
