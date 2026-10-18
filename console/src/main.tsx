import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App.js";
import { captureFromLocation, inviteStore } from "./invite.mjs";
import "./styles.css";

// Before anything renders or requests: an invitation token in the address
// bar moves into this tab's session storage and out of the URL.
captureFromLocation(window.location, window.history, inviteStore(), Date.now());
// A link opened in a tab already showing the invitation page changes only
// the fragment, which loads no document: capture it the same way, then start
// a fresh document so the page shows the invitation just opened.
window.addEventListener("hashchange", () => {
  if (captureFromLocation(window.location, window.history, inviteStore(), Date.now())) {
    window.location.reload();
  }
});

const root = document.getElementById("root");
if (!root) {
  throw new Error("index.html must carry #root");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
