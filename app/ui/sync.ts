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
const paceLine = find("sync-pace", HTMLElement);

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
      return "Add a device";
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
      return "Your shortcuts stay on this device until you enable sync.";
    case "noAccount":
      return "Open Devices to add another device or join with an invitation code.";
    case "waiting":
      return "The first sync starts in a moment.";
    case "syncing":
      return "Syncing now…";
    case "synced":
      return sync.last === null ? "Synced." : `Last synced ${ago(sync.last)}.`;
    case "failed":
      return `The last sync failed: ${(sync.error ?? "for no reason given").replace(/\baccounts\b/giu, "libraries").replace(/\baccount\b/giu, "library")}.`;
  }
}

/** When the window writes this device's bundle and when it reads the others', which only a window that syncs can say. */
function pace(sync: SyncView): string {
  if (sync.state === "off" || sync.state === "noAccount") {
    return "";
  }
  const writes = "The window sends this device's changes after an edit here.";
  return sync.watched
    ? `${writes} It reads the other devices' changes as soon as the folder changes, and every five minutes.`
    : `${writes} It reads the other devices' changes every five minutes.`;
}

let editing = false;

/** Draws the syncing, and wires its controls to send what a person chooses. */
export function drawSync(sync: SyncView, send: (intent: Intent) => void): void {
  state.replaceChildren(icon("sync"), el("span", "", short(sync)));
  state.classList.toggle("failed", sync.state === "failed");
  state.title = [long(sync), pace(sync)].filter((sentence) => sentence !== "").join(" ");
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
  paceLine.textContent = pace(sync);
  paceLine.hidden = paceLine.textContent === "";
}
