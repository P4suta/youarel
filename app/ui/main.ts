// The window's side of the launcher: it forwards keys and text to the interface machine and draws the view it gets back.
// It decides nothing; every row's text reaches the page as text, never as markup.

interface RowView {
  readonly kind: "parent" | "add" | "folder" | "shortcut";
  readonly name: string;
  readonly detail: string;
  readonly alias: string;
  /** The shortcut's icon as a `data:` address of its PNG, which the page's policy admits as an image alone. */
  readonly icon: string | null;
}

interface View {
  readonly header: string;
  readonly prompt: string;
  readonly rows: readonly RowView[];
  readonly selected: number;
  readonly footer: string;
  readonly carrying: boolean;
}

interface Press {
  readonly key: string;
  readonly control: boolean;
  readonly shift: boolean;
}

interface Tauri {
  readonly core: {
    invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  };
}

declare global {
  interface Window {
    readonly __TAURI__: Tauri;
  }
}

/** The keys the machine reads by name, which the input field never sees. */
const NAMED: ReadonlySet<string> = new Set([
  "Enter",
  "Escape",
  "Backspace",
  "Tab",
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
]);
/** A letter, which the control key turns into a command. */
const LETTER = /^[a-z]$/i;

function element<T extends HTMLElement>(id: string, kind: new () => T): T {
  const found = document.getElementById(id);
  if (!(found instanceof kind)) {
    throw new Error(`the page lacks #${id}`);
  }
  return found;
}

const header = element("header", HTMLElement);
const prompt = element("prompt", HTMLSpanElement);
const field = element("field", HTMLInputElement);
const rows = element("rows", HTMLUListElement);
const footer = element("footer", HTMLElement);

let carrying = false;
let composing = false;
/** The calls in flight, in order, so the machine reads the keys as the person pressed them. */
let queue: Promise<void> = Promise.resolve();

function cell(kind: string, text: string): HTMLSpanElement {
  const span = document.createElement("span");
  span.className = kind;
  span.textContent = text;
  return span;
}

/** The icon of a row, or an empty place of the same size that keeps the names aligned. */
function picture(row: RowView): HTMLElement {
  if (row.icon === null) {
    return cell("icon", "");
  }
  const image = document.createElement("img");
  image.className = "icon";
  image.alt = "";
  image.src = row.icon;
  return image;
}

function draw(view: View): void {
  header.textContent = view.header;
  prompt.textContent = view.prompt;
  footer.textContent = view.footer;
  carrying = view.carrying;
  const items = view.rows.map((row, index) => {
    const item = document.createElement("li");
    item.className = row.kind;
    item.setAttribute("aria-selected", String(index === view.selected));
    item.append(picture(row), cell("name", row.name), cell("detail", row.detail));
    if (row.alias !== "") {
      item.append(cell("alias", `[${row.alias}]`));
    }
    return item;
  });
  rows.replaceChildren(...items);
  items[view.selected]?.scrollIntoView({ block: "nearest" });
}

/** Calls a command after every earlier one, and draws the view it answers with. */
function send(command: string, args: Record<string, unknown> = {}): void {
  queue = queue.then(async () => {
    try {
      const view = await window.__TAURI__.core.invoke<View | null>(command, args);
      if (view !== null) {
        draw(view);
      }
    } catch (error: unknown) {
      footer.textContent = String(error);
    }
  });
}

/** Whether the machine reads a press, rather than the input field. */
function commands(press: Press): boolean {
  if (NAMED.has(press.key)) {
    return true;
  }
  if (!press.control || !LETTER.test(press.key)) {
    return false;
  }
  // Control and V moves a carried entry, and pastes text otherwise.
  return press.key.toLowerCase() !== "v" || carrying;
}

field.addEventListener("keydown", (event) => {
  if (event.isComposing || composing) {
    return;
  }
  const press: Press = { key: event.key, control: event.ctrlKey, shift: event.shiftKey };
  if (commands(press)) {
    event.preventDefault();
    send("press", { press });
  }
});

field.addEventListener("beforeinput", (event) => {
  if (event.inputType === "insertText" && !composing && event.data !== null) {
    event.preventDefault();
    send("typed", { text: event.data });
  }
});

field.addEventListener("compositionstart", () => {
  composing = true;
});

field.addEventListener("compositionend", (event) => {
  composing = false;
  field.value = "";
  send("typed", { text: event.data });
});

field.addEventListener("paste", (event) => {
  event.preventDefault();
  send("paste", { text: event.clipboardData?.getData("text/plain") ?? "" });
});

window.addEventListener("focus", () => {
  field.focus();
  send("view");
});

field.focus();
send("view");

export {};
