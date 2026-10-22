// Draws where the window's syncing stands: a line in the sidebar's footer, and the sync section of the settings.

import { el, find, icon } from "./dom.js";
import type { Intent, SyncView } from "./types.js";

const SECOND_MS = 1000;
const MINUTE_MS = 60 * SECOND_MS;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;
const RELATIVE = new Intl.RelativeTimeFormat("en", { numeric: "auto" });

const state = find("sync-state", HTMLElement);
const enabled = find("sync-enabled", HTMLInputElement);
const form = find("sync-form", HTMLFormElement);
const target = find("sync-target", HTMLInputElement);
const line = find("sync-line", HTMLElement);

/** How long ago a moment was, in words. */
export function ago(time: number): string {
  const elapsed = time - Date.now();
  if (-elapsed < MINUTE_MS) {
    return "just now";
  }
  if (-elapsed < HOUR_MS) {
    return RELATIVE.format(Math.round(elapsed / MINUTE_MS), "minute");
  }
  if (-elapsed < DAY_MS) {
    return RELATIVE.format(Math.round(elapsed / HOUR_MS), "hour");
  }
  return RELATIVE.format(Math.round(elapsed / DAY_MS), "day");
}

/** The words of the sidebar's footer for each state. */
function short(sync: SyncView): string {
  switch (sync.state) {
    case "off":
      return "On this device";
    case "noAccount":
      return "No account";
    case "waiting":
      return "Waiting to sync";
    case "syncing":
      return "Syncing…";
    case "synced":
      return "Synced";
    case "failed":
      return "Sync failed";
  }
}

/** The sentence of the settings for each state. */
function long(sync: SyncView): string {
  switch (sync.state) {
    case "off":
      return "The window keeps your shortcuts on this device; “youarel sync” still syncs by hand.";
    case "noAccount":
      return "This device belongs to no account yet: run “youarel account create”, or “youarel pair join” with a code from one of your devices.";
    case "waiting":
      return "The first sync starts in a moment.";
    case "syncing":
      return "Syncing now…";
    case "synced":
      return sync.last === null ? "Synced." : `Last synced ${ago(sync.last)}.`;
    case "failed":
      return `The last sync failed: ${sync.error ?? "for no reason given"}.`;
  }
}

let editing = false;

/** Draws the syncing, and wires its controls to send what a person chooses. */
export function drawSync(sync: SyncView, send: (intent: Intent) => void): void {
  state.replaceChildren(icon("sync"), el("span", "", short(sync)));
  state.classList.toggle("failed", sync.state === "failed");
  state.title = long(sync);
  state.onclick = () => send({ kind: "syncNow" });
  enabled.checked = sync.enabled;
  enabled.onchange = () => send({ kind: "syncSettings", enabled: enabled.checked, target: target.value });
  if (!editing) {
    target.value = sync.target;
  }
  target.onfocus = () => {
    editing = true;
  };
  target.onblur = () => {
    editing = false;
  };
  form.onsubmit = (event) => {
    event.preventDefault();
    editing = false;
    send({ kind: "syncSettings", enabled: enabled.checked, target: target.value });
  };
  line.textContent = long(sync);
  line.classList.toggle("error-line", sync.state === "failed");
}
