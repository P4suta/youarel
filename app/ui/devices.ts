import { button, el, find } from "./dom.js";
import { codeImage, drawCodeImage } from "./qr.js";
import type { CodeImage } from "./qr.js";
import type { AskSheet } from "./sheets.js";
import { invoke, listen } from "./types.js";

type Phase = "idle" | "preparing" | "waiting" | "confirming" | "syncing" | "done" | "expired" | "refused" | "cancelled";
interface Progress {
  readonly token: number;
  readonly phase: Phase;
  readonly busy: boolean;
  readonly code: string | null;
  readonly qr: CodeImage | null;
  readonly error: string | null;
  readonly recoveryReady: boolean;
  readonly recoveryAcknowledgment: boolean;
}
interface Device {
  readonly id: string;
  readonly short: string;
  readonly name: string;
  readonly own: boolean;
  readonly added: number | null;
  readonly recovery: boolean;
}
interface Display {
  readonly snapshot: { readonly thisDevice: string; readonly joined: boolean; readonly member: boolean; readonly devices: readonly Device[] };
  readonly progress: Progress;
}
type Request =
  | { readonly kind: "invite"; readonly target: string }
  | { readonly kind: "join"; readonly target: string; readonly code: string }
  | { readonly kind: "cancelPairing" }
  | { readonly kind: "nameDevice"; readonly id: string; readonly name: string }
  | { readonly kind: "removeDevice"; readonly id: string; readonly confirmed: boolean }
  | { readonly kind: "showRecoveryKey" }
  | { readonly kind: "acknowledgeRecovery" }
  | { readonly kind: "recover"; readonly target: string; readonly key: string };

const PHASES = new Set<string>(["idle", "preparing", "waiting", "confirming", "syncing", "done", "expired", "refused", "cancelled"]);
const ID = /^[0-9a-f]{64}$/u;
const MOST_RECOVERY_TEXT = 512;

function object(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null; }
function text(value: unknown): value is string | null { return value === null || typeof value === "string"; }
function moment(value: unknown): value is number | null { return value === null || (typeof value === "number" && Number.isSafeInteger(value) && value >= 0); }
function device(value: unknown): value is Device {
  return object(value) && typeof value["id"] === "string" && ID.test(value["id"])
    && typeof value["short"] === "string" && typeof value["name"] === "string"
    && typeof value["own"] === "boolean" && typeof value["recovery"] === "boolean" && moment(value["added"]);
}
function progress(value: unknown): value is Progress {
  return object(value) && typeof value["token"] === "number" && Number.isSafeInteger(value["token"])
    && typeof value["phase"] === "string" && PHASES.has(value["phase"])
    && typeof value["busy"] === "boolean" && text(value["code"]) && text(value["error"])
    && (value["qr"] === null || (typeof value["code"] === "string" && codeImage(value["qr"])))
    && typeof value["recoveryReady"] === "boolean" && typeof value["recoveryAcknowledgment"] === "boolean";
}
function display(value: unknown): value is Display {
  if (!object(value) || !object(value["snapshot"]) || !progress(value["progress"])) { return false; }
  const snapshot = value["snapshot"];
  return typeof snapshot["thisDevice"] === "string" && ID.test(snapshot["thisDevice"])
    && typeof snapshot["joined"] === "boolean" && typeof snapshot["member"] === "boolean"
    && Array.isArray(snapshot["devices"]) && snapshot["devices"].every(device);
}
function words(value: Progress): string {
  if (value.error !== null) { return value.error; }
  switch (value.phase) {
    case "idle": return "Add a device to share this library.";
    case "preparing": return "Preparing…";
    case "waiting": return "Waiting for the other device…";
    case "confirming": return "Confirming the other device…";
    case "syncing": return "Device added. Syncing shortcuts…";
    case "done": return "Done.";
    case "expired": return "The invitation expired. Start again with a new code.";
    case "refused": return "Pairing stopped. Check the code and sync location, then try again.";
    case "cancelled": return value.busy ? "Stopping the invitation…" : "Invitation cancelled.";
  }
}

export class DevicesPane {
  private readonly settings = find("settings", HTMLDialogElement);
  private readonly list = find("device-list", HTMLElement);
  private readonly line = find("device-line", HTMLElement);
  private readonly target = find("device-target", HTMLInputElement);
  private readonly invitation = find("device-invitation", HTMLDialogElement);
  private readonly code = find("device-code", HTMLElement);
  private readonly qr = find("device-qr", HTMLElement);
  private readonly pairingLine = find("device-pairing-line", HTMLElement);
  private readonly cancel = find("device-cancel", HTMLButtonElement);
  private readonly recovery = find("device-recovery", HTMLDialogElement);
  private readonly key = find("device-recovery-key", HTMLElement);
  private readonly recoveryStatus = find("device-recovery-status", HTMLElement);
  private serial = 0;
  private readingKey = false;
  private current: Display | null = null;

  constructor(private readonly ask: AskSheet, private readonly failed: (error: unknown) => void) {
    find("device-invite", HTMLButtonElement).onclick = () => void this.invite();
    find("device-join", HTMLButtonElement).onclick = () => void this.join();
    find("device-recover", HTMLButtonElement).onclick = () => void this.recover();
    find("device-new-recovery", HTMLButtonElement).onclick = () => void this.createRecovery();
    find("device-retry", HTMLButtonElement).onclick = () => void this.refresh();
    find("device-copy-code", HTMLButtonElement).onclick = () => void this.copy(this.code.textContent ?? "");
    this.cancel.onclick = () => void this.send({ kind: "cancelPairing" });
    find("device-pairing-close", HTMLButtonElement).onclick = () => this.invitation.close();
    this.invitation.addEventListener("cancel", (event) => {
      if (this.current?.progress.busy === true) { event.preventDefault(); void this.send({ kind: "cancelPairing" }); }
    });
    this.recovery.addEventListener("cancel", (event) => event.preventDefault());
    find("device-copy-recovery", HTMLButtonElement).onclick = () => void this.copy(this.key.textContent ?? "");
    find("device-print-recovery", HTMLButtonElement).onclick = () => {
      if (this.recovery.open && this.key.textContent !== "") { window.print(); }
    };
    find("device-saved-recovery", HTMLButtonElement).onclick = () => void this.saved();
    void listen("devices-changed", () => void this.refresh()).catch(this.failed);
  }

  async refresh(): Promise<void> {
    if (!this.settings.open && !this.invitation.open) { return; }
    const serial = ++this.serial;
    try {
      const value = await invoke<unknown>("devices");
      if (serial !== this.serial) { return; }
      if (!display(value)) { throw new Error("The device list has an invalid response."); }
      this.current = value;
      this.draw(value);
      if (value.progress.recoveryReady && !this.readingKey && !this.recovery.open) { await this.showKey(); }
    } catch (error: unknown) {
      if (serial !== this.serial) { return; }
      this.code.textContent = "";
      this.code.hidden = true;
      drawCodeImage(this.qr, null);
      find("device-copy-code", HTMLButtonElement).hidden = true;
      this.failed(error);
    }
  }

  private draw(value: Display): void {
    const status = value.snapshot.joined && !value.snapshot.member
      ? "Sync stopped on this device because another device removed it."
      : words(value.progress);
    this.line.textContent = status;
    this.pairingLine.textContent = status;
    this.code.textContent = value.progress.code ?? "";
    this.code.hidden = value.progress.code === null;
    drawCodeImage(this.qr, value.progress.qr);
    find("device-copy-code", HTMLButtonElement).hidden = value.progress.code === null;
    this.cancel.hidden = !value.progress.busy || value.progress.phase === "syncing" || value.progress.phase === "cancelled";
    find("device-pairing-close", HTMLButtonElement).hidden = value.progress.busy;
    const locked = value.progress.busy || value.progress.recoveryAcknowledgment;
    for (const id of ["device-invite", "device-join", "device-recover", "device-new-recovery"]) { find(id, HTMLButtonElement).disabled = locked; }
    find("device-invite", HTMLButtonElement).disabled = locked || (value.snapshot.joined && !value.snapshot.member);
    find("device-join", HTMLButtonElement).disabled = locked || value.snapshot.joined;
    find("device-recover", HTMLButtonElement).disabled = locked || value.snapshot.joined;
    find("device-new-recovery", HTMLButtonElement).disabled = locked || !value.snapshot.member;
    if (this.target !== document.activeElement && this.target.value === "") { this.target.value = find("sync-target", HTMLInputElement).value; }
    this.list.replaceChildren(...value.snapshot.devices.map((held) => this.row(held, locked)));
  }

  private row(held: Device, locked: boolean): HTMLElement {
    const row = el("div", "device-row");
    const label = held.name || (held.recovery ? "Recovery key" : held.own ? "This device" : "Device");
    const details = el("div", "device-details");
    details.append(el("strong", "", label), el("code", "", held.short));
    if (held.own) { details.append(el("span", "hint", "This device")); }
    if (held.added !== null) { details.append(el("span", "hint", `Added ${new Date(held.added).toLocaleDateString()}`)); }
    const actions = el("div", "buttons");
    const name = button("Name…", "secondary");
    name.disabled = locked;
    name.onclick = () => void this.name(held);
    actions.append(name);
    if (!held.own) {
      const remove = button("Remove…", "secondary danger");
      remove.disabled = locked || this.current?.snapshot.member !== true;
      remove.onclick = () => void this.remove(held, label);
      actions.append(remove);
    }
    row.append(details, actions);
    return row;
  }

  private async send(request: Request): Promise<void> {
    try { await invoke<unknown>("device_action", { request }); await this.refresh(); }
    catch (error: unknown) { this.failed(error); }
  }
  private location(): string | null {
    const target = this.target.value.trim();
    if (target === "") { this.failed("Choose a shared folder or relay first."); this.target.focus(); return null; }
    return target;
  }
  private async invite(): Promise<void> {
    const target = this.location();
    if (target === null) { return; }
    if (!this.invitation.open) { this.invitation.showModal(); }
    await this.send({ kind: "invite", target });
  }
  private async join(): Promise<void> {
    const target = this.location();
    if (target === null) { return; }
    const code = await this.ask.ask({ title: "Join from another device", text: "Enter the invitation code shown on your other device.", action: "Join", answer: "" });
    if (code === null) { return; }
    this.invitation.showModal();
    await this.send({ kind: "join", target, code });
  }
  private async name(held: Device): Promise<void> {
    const name = await this.ask.ask({ title: "Name this device", text: "Use a short name. Leave it empty to clear the name.", action: "Save", answer: held.name });
    if (name !== null) { await this.send({ kind: "nameDevice", id: held.id, name }); }
  }
  private async remove(held: Device, label: string): Promise<void> {
    const text = held.recovery ? "This recovery key will stop working." : "This device will stop receiving new changes. Its existing shortcuts stay on it.";
    const confirmed = await this.ask.ask({ title: `Remove ${label}?`, text, action: "Remove", answer: null });
    if (confirmed !== null) { await this.send({ kind: "removeDevice", id: held.id, confirmed: true }); }
  }
  private async createRecovery(): Promise<void> {
    const confirmed = await this.ask.ask({ title: "Create a recovery key?", text: "The next sheet shows the new key once. Keep it somewhere safe so you can recover your library.", action: "Create key", answer: null });
    if (confirmed !== null) { await this.send({ kind: "showRecoveryKey" }); }
  }
  private async recover(): Promise<void> {
    const target = this.location();
    if (target === null) { return; }
    const key = await this.ask.ask({ title: "Recover this library", text: "Enter your recovery key. The shared folder or relay must hold your library's changes.", action: "Recover", answer: "", secret: true });
    if (key !== null) { await this.send({ kind: "recover", target, key }); }
  }
  private async copy(value: string): Promise<void> {
    if (value === "") { return; }
    try { await invoke<void>("copy", { text: value }); } catch (error: unknown) { this.failed(error); }
  }
  private async showKey(): Promise<void> {
    this.readingKey = true;
    try {
      const value = await invoke<unknown>("take_recovery_key");
      if (typeof value !== "string" || value.length === 0 || value.length > MOST_RECOVERY_TEXT) { throw new Error("The recovery key is unavailable."); }
      this.key.textContent = value;
      this.recoveryStatus.textContent = "";
      this.recovery.showModal();
    } finally { this.readingKey = false; }
  }
  private async saved(): Promise<void> {
    try {
      await invoke<unknown>("device_action", { request: { kind: "acknowledgeRecovery" } });
      this.key.textContent = "";
      this.recovery.close();
      await this.refresh();
    } catch (error: unknown) { this.recoveryStatus.textContent = String(error); }
  }
}
