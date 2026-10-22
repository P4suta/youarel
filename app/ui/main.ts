// The window's side of the library: it sends what a person does to the library and draws the view it gets back.
// It decides nothing the library decides; every shortcut's text reaches the page as text, never as markup.

import { COLUMN_TITLES, DRAGGED, DRAGGED_FOLDERS, cards, columns, extend, layouts, list, shortcuts, sidebar } from "./draw.js";
import type { Handlers } from "./draw.js";
import { button, el, find, icon } from "./dom.js";
import { AddSheet, AskSheet, EditSheet, MarkSheet, Menu } from "./sheets.js";
import type { MenuChoice } from "./sheets.js";
import { drawSync } from "./sync.js";
import { themePane, wear } from "./theme.js";
import { invoke, listen, listenText } from "./types.js";
import type { Browser, ColumnEntry, ColumnName, EmojiGroup, Intent, Item, Key, Layout, Sort, ThemeOption, View } from "./types.js";

/** How long a notice stays before it fades. */
const NOTICE_MS = 4000;
/** The layouts that the command key and a digit choose. */
const LAYOUT_KEYS: Readonly<Record<string, Layout>> = { "1": "cards", "2": "list", "3": "columns" };
/** The keys that move the selection, by the names the DOM gives them. */
const MOVES: Readonly<Record<string, Key>> = {
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  Home: "first",
  End: "last",
};
/** The event the library raises when a person summons the window. */
const SUMMONED = "summoned";
/** The event the library raises when a sync begins or ends. */
const SYNCED = "synced";
/** The event the library raises when shortcuts got their icons. */
const ICONED = "iconed";
/** The event the library raises with a command a person chose from the app's menu. */
const COMMAND = "command";

const nav = find("scopes", HTMLElement);
const title = find("title", HTMLElement);
const count = find("count", HTMLElement);
const layoutGroup = find("layouts", HTMLElement);
const search = find("search", HTMLInputElement);
const sort = find("sort", HTMLSelectElement);
const grouping = find("group", HTMLSelectElement);
const content = find("content", HTMLElement);
const notice = find("notice", HTMLElement);
const dropHint = find("drop-hint", HTMLElement);
const addButton = find("add-button", HTMLButtonElement);
const settingsButton = find("settings-button", HTMLButtonElement);
const settings = find("settings", HTMLDialogElement);
const importHtml = find("import-html", HTMLButtonElement);
const importFile = find("import-file", HTMLInputElement);
const themeSection = settings.querySelector<HTMLElement>('[data-pane="theme"]');
const browserButtons = find("browser-buttons", HTMLElement);
const trailNav = find("trail", HTMLElement);
const newFolderButton = find("new-folder", HTMLButtonElement);
/** What stands between the steps of the trail. */
const TRAIL_SEPARATOR = "›";
/** The name of the top of the library, which the trail of the cards and the list starts from. */
const ALL = "All";
/** The columns of the list a person may hide, in the order the list shows them. */
const HIDEABLE: readonly ColumnName[] = ["link", "collection", "added"];
/** Where the folder above the one the cards and the list stand in sits in the trail, counted back from its end. */
const PARENT_STEP = -2;
/** The name a person reads for each browser. */
const BROWSER_TITLES: Readonly<Record<Browser, string>> = { safari: "Safari", chrome: "Chrome", firefox: "Firefox" };

let current: View | null = null;
/** The calls in flight, in order, so the library reads what a person did in the order they did it. */
let queue: Promise<void> = Promise.resolve();
let composing = false;
let noticeTimer: number | undefined;
/** The icons drawn so far by name, and the names asked for. */
const drawnIcons = new Map<string, string>();
const askedIcons = new Set<string>();
/** The images waiting for each icon. */
const waiting = new Map<string, HTMLImageElement[]>();

function showNotice(view: View): void {
  window.clearTimeout(noticeTimer);
  if (view.notice === null) {
    notice.hidden = true;
    return;
  }
  notice.textContent = view.notice.text;
  notice.classList.toggle("error", view.notice.error);
  notice.hidden = false;
  noticeTimer = window.setTimeout(() => {
    notice.hidden = true;
  }, NOTICE_MS);
}

/** Asks the library for the icons the page lacks, and fills every image that waits for one. */
async function fetchIcons(): Promise<void> {
  const names = [...waiting.keys()].filter((name) => !askedIcons.has(name));
  if (names.length === 0) {
    return;
  }
  for (const name of names) {
    askedIcons.add(name);
  }
  const found = await invoke<Record<string, string>>("icons", { names });
  for (const [name, address] of Object.entries(found)) {
    drawnIcons.set(name, address);
    for (const image of waiting.get(name) ?? []) {
      image.src = address;
    }
    waiting.delete(name);
  }
}

/** Draws the theme pane of the settings while they show, from every theme the library knows. */
async function drawThemes(): Promise<void> {
  const theme = current?.theme ?? null;
  if (themeSection === null || theme === null || !settings.open) {
    return;
  }
  const themes = await invoke<ThemeOption[]>("themes");
  themePane(themeSection, themes, theme, {
    choose: (follow, light, dark) => send({ kind: "theme", follow, light, dark }),
    import: (text) => send({ kind: "importTheme", text }),
    remove: (id) => send({ kind: "removeTheme", id }),
  });
}

function openSettings(): void {
  settings.showModal();
  void drawThemes();
}

/** Draws the trail of the folders from the top down to the one the cards and the list stand in, each a step up to it. */
function drawFolderTrail(view: View): void {
  const top = button(ALL, "");
  top.addEventListener("click", () => send({ kind: "scope", scope: { kind: "all" } }));
  const last = view.trail.length - 1;
  const steps = view.trail.flatMap((step, place) => {
    if (place === last) {
      return [el("span", "separator", TRAIL_SEPARATOR), el("span", "here", step.name)];
    }
    const made = button(step.name, "");
    made.addEventListener("click", () => send({ kind: "scope", scope: { kind: "collection", id: step.id } }));
    return [el("span", "separator", TRAIL_SEPARATOR), made];
  });
  trailNav.replaceChildren(top, ...steps);
}

/** Steps up to the folder above the one the cards and the list stand in, as an explorer's command and up arrow do. */
function up(): void {
  if (current === null || current.layout === "columns" || current.trail.length === 0) {
    return;
  }
  const parent = current.trail.at(PARENT_STEP);
  send({ kind: "scope", scope: parent === undefined ? { kind: "all" } : { kind: "collection", id: parent.id } });
}

/** Draws the trail of the folders the columns open, after the scope's heading, each a step back to it. */
function drawTrail(view: View): void {
  if (view.trail.length === 0) {
    trailNav.replaceChildren();
    return;
  }
  if (view.layout !== "columns") {
    drawFolderTrail(view);
    return;
  }
  const root = button(view.title, "");
  root.addEventListener("click", () => send({ kind: "scope", scope: view.sidebar.scope }));
  const steps = view.trail.flatMap((step) => {
    const made = button(step.name, "");
    made.addEventListener("click", () => send({ kind: "column", depth: step.depth, entry: { kind: "folder", id: step.id } }));
    return [el("span", "separator", TRAIL_SEPARATOR), made];
  });
  trailNav.replaceChildren(root, ...steps);
}

/** Scrolls an entry's column up or down until the entry shows, and leaves the columns' sideways scroll alone, which a scroll into view would move too. */
function keepInColumn(entry: HTMLElement): void {
  const column = entry.closest<HTMLElement>(".column");
  if (column === null) {
    return;
  }
  const box = entry.getBoundingClientRect();
  const frame = column.getBoundingClientRect();
  if (box.top < frame.top) {
    column.scrollTop -= frame.top - box.top;
  } else if (box.bottom > frame.bottom) {
    column.scrollTop += box.bottom - frame.bottom;
  }
}

/** Draws a view; one that answers no intent of the person's keeps the scroll and the notice on screen, unless the notice is new. */
function draw(view: View, answers = true): void {
  const shown = current?.notice?.text;
  current = view;
  wear(view.theme);
  void drawThemes();
  drawSync(view.sync, send);
  waiting.clear();
  sidebar(nav, view, handlers);
  title.textContent = view.title;
  count.textContent = shortcuts(view.count);
  layouts(layoutGroup, view.layout, (layout) => send({ kind: "layout", layout }));
  browserButtons.replaceChildren(
    ...view.browsers.map((browser) => {
      const choice = button(BROWSER_TITLES[browser], "secondary");
      choice.addEventListener("click", () => send({ kind: "importBrowser", browser }));
      return choice;
    }),
  );
  if (document.activeElement !== search && !composing) {
    search.value = view.search;
  }
  sort.value = view.sort;
  grouping.value = view.grouping;
  drawTrail(view);
  const drawn = view.layout === "cards" ? cards(view, handlers) : view.layout === "list" ? list(view, handlers) : columns(view, handlers);
  content.replaceChildren(drawn);
  if (answers && view.layout === "columns") {
    for (const shown of content.querySelectorAll<HTMLElement>('.column [aria-selected="true"]')) {
      keepInColumn(shown);
    }
    // The deepest column and its preview stay at the right edge as the walk goes deeper or turns back up, as an explorer's columns do.
    content.scrollLeft = content.scrollWidth;
  } else if (answers) {
    content.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }
  if (answers || view.notice?.text !== shown) {
    showNotice(view);
  }
  void fetchIcons();
}

/** Sends an intent after every earlier one, and draws the view it answers with. */
function send(intent: Intent): void {
  queue = queue.then(async () => {
    try {
      draw(await invoke<View>("act", { intent }));
    } catch (error: unknown) {
      notice.textContent = String(error);
      notice.classList.add("error");
      notice.hidden = false;
    }
  });
}

/** Draws the view as it stands after every earlier intent, as when a sync began or ended. */
function refresh(): void {
  queue = queue.then(async () => {
    try {
      draw(await invoke<View>("view"), false);
    } catch (error: unknown) {
      notice.textContent = String(error);
      notice.classList.add("error");
      notice.hidden = false;
    }
  });
}

/** The focused shortcut, from the listing or the preview. */
function focused(): Item | null {
  if (current === null || current.focus === null) {
    return null;
  }
  const focus = current.focus;
  return current.items.find((item) => item.id === focus) ?? (current.preview?.id === focus ? current.preview : null);
}

/** How many cards a row of the grid holds now. */
function across(): number {
  const grid = content.querySelector(".cards");
  if (!(grid instanceof HTMLElement)) {
    return 1;
  }
  return getComputedStyle(grid).gridTemplateColumns.split(" ").length;
}

async function copy(link: string): Promise<void> {
  await invoke("copy", { text: link });
  notice.textContent = "Copied the URL";
  notice.classList.remove("error");
  notice.hidden = false;
}

const ask = new AskSheet();
const marking = new MarkSheet((query) => invoke<EmojiGroup[]>("emoji", { query }));
const menu = new Menu();

/** How many folders, in words. */
function folders(count: number): string {
  return count === 1 ? "1 folder" : `${count} folders`;
}

async function confirmRemoval(ids: readonly string[]): Promise<void> {
  const held = new Set(current?.folders.map((folder) => folder.id) ?? []);
  const folderCount = ids.filter((id) => held.has(id)).length;
  const linkCount = ids.length - folderCount;
  const named =
    folderCount === 0
      ? shortcuts(linkCount)
      : linkCount === 0
        ? folders(folderCount)
        : `${folders(folderCount)} and ${shortcuts(linkCount)}`;
  const answer = await ask.ask({
    title: ids.length === 1 && folderCount === 0 ? "Remove this shortcut?" : `Remove ${named}?`,
    text:
      folderCount === 0
        ? "Every device of the account removes it too."
        : "The shortcuts and collections inside go with them, and every device of the account removes them too.",
    action: "Remove",
    answer: null,
  });
  if (answer !== null) {
    send({ kind: "remove", ids });
  }
}

const adding = new AddSheet({
  clipboard: () => invoke<string | null>("clipboard"),
  title: (link) => invoke<string | null>("title", { link }),
  add: async (link, name, collection) => {
    try {
      const view = await invoke<View>("add", { link, title: name, collection });
      draw(view);
      return view.notice?.error !== true;
    } catch (error: unknown) {
      notice.textContent = String(error);
      notice.classList.add("error");
      notice.hidden = false;
      return false;
    }
  },
});

const editing = new EditSheet(
  (fields) => send({ kind: "edit", fields }),
  (id) => void confirmRemoval([id]),
);

/** Opens the add sheet over the collection the window stands in. */
function openAdd(dropped: string | null): void {
  if (current !== null) {
    void adding.open(current.collections, current.here, dropped);
  }
}

function edit(item: Item): void {
  if (current !== null) {
    editing.open(item, current.collections);
  }
}

/** Asks for a name and makes a collection inside `parent`, or at the top for null. */
async function newFolder(parent: string | null): Promise<void> {
  const place = current?.collections.find((choice) => choice.id === parent)?.path;
  const name = await ask.ask({
    title: place === undefined ? "New collection" : `New collection in ${place}`,
    text: "",
    action: "Create",
    answer: "",
  });
  if (name !== null) {
    send({ kind: "newCollection", name, parent });
  }
}

const handlers: Handlers = {
  scope: (scope) => send({ kind: "scope", scope }),
  select: (id, how) => send({ kind: "select", id, extend: how }),
  open: (id) => send({ kind: "open", id }),
  menu: (event, item) => {
    const selected = current?.selected.includes(item.id) === true ? (current?.selected ?? [item.id]) : [item.id];
    if (current?.selected.includes(item.id) !== true) {
      send({ kind: "select", id: item.id, extend: "replace" });
    }
    const choices: MenuChoice[] = [
      { label: "Open", act: () => send({ kind: "open", id: item.id }) },
      { label: "Copy URL", act: () => void copy(item.link) },
      "line",
      { label: "Edit…", act: () => edit(item) },
      "line",
      { label: selected.length === 1 ? "Remove…" : `Remove ${shortcuts(selected.length)}…`, danger: true, act: () => void confirmRemoval(selected) },
    ];
    menu.show(event, choices);
  },
  collectionMenu: (event, id, name, mark) => {
    menu.show(event, [
      { label: "New collection inside…", act: () => void newFolder(id) },
      "line",
      {
        label: "Set Mark…",
        act: () =>
          void marking.open(name, mark).then((chosen) => {
            if (chosen !== null) {
              send({ kind: "markCollection", id, mark: chosen });
            }
          }),
      },
      {
        label: "Rename…",
        act: () =>
          void ask.ask({ title: "Rename collection", text: "", action: "Rename", answer: name }).then((renamed) => {
            if (renamed !== null) {
              send({ kind: "renameCollection", id, name: renamed });
            }
          }),
      },
      "line",
      {
        label: "Remove…",
        danger: true,
        act: () =>
          void ask
            .ask({ title: `Remove ${name}?`, text: "The shortcuts and collections inside it go with it.", action: "Remove", answer: null })
            .then((answer) => {
              if (answer !== null) {
                send({ kind: "removeCollection", id });
              }
            }),
      },
    ]);
  },
  newCollection: () => void newFolder(null),
  dropOnCollection: (ids, collection) => send({ kind: "file", ids, collection }),
  moveFolders: (ids, collection) => send({ kind: "moveFolders", ids, collection }),
  retitle: (id, typed) => send({ kind: "retitle", id, title: typed }),
  renameFolder: (id, name) => send({ kind: "renameCollection", id, name }),
  saveField: (item, field, value) =>
    send({
      kind: "edit",
      fields: {
        id: item.id,
        title: item.title,
        alias: field === "alias" ? value.trim() : item.alias,
        note: field === "note" ? value : item.note,
        collection: item.collectionId,
      },
    }),
  column: (depth: number, entry: ColumnEntry) => send({ kind: "column", depth, entry: { kind: entry.kind, id: entry.id } }),
  copy: (link) => void copy(link),
  edit,
  sort: (chosen: Sort) => send({ kind: "sort", sort: chosen }),
  columnMenu: (event) => {
    const hidden = new Set(current?.hiddenColumns ?? []);
    menu.show(
      event,
      HIDEABLE.map((column) => ({
        label: COLUMN_TITLES[column],
        checked: !hidden.has(column),
        act: () => send({ kind: "showColumn", column, shown: hidden.has(column) }),
      })),
    );
  },
  disclose: (id, open) => send({ kind: "disclose", id, open }),
  discloseSidebar: (id, open) => send({ kind: "discloseSidebar", id, open }),
  layout: (layout) => send({ kind: "layout", layout }),
  icon: (image, name) => {
    const address = drawnIcons.get(name);
    if (address !== undefined) {
      image.src = address;
      return;
    }
    waiting.set(name, [...(waiting.get(name) ?? []), image]);
  },
};

/** Whether a key press goes to a field the person types into. */
function typing(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement;
}

/** The commands typed with the command key, or the control key off macOS. */
function command(event: KeyboardEvent): boolean {
  const key = event.key.toLowerCase();
  const layout = LAYOUT_KEYS[key];
  if (layout !== undefined) {
    send({ kind: "layout", layout });
  } else if (key === "arrowup" && !typing(event.target)) {
    up();
  } else if (key === "arrowdown" && !typing(event.target)) {
    send({ kind: "open", id: null });
  } else if (key === "k" || key === "f") {
    search.focus();
    search.select();
  } else if (key === "n" && event.shiftKey) {
    void newFolder(current?.here ?? null);
  } else if (key === "n") {
    openAdd(null);
  } else if (key === ",") {
    openSettings();
  } else if (key === "e") {
    const item = focused();
    if (item !== null) {
      edit(item);
    }
  } else if (key === "a" && !typing(event.target)) {
    send({ kind: "selectAll" });
  } else if (key === "c" && !typing(event.target)) {
    const item = focused();
    if (item !== null) {
      void copy(item.link);
    }
  } else {
    return false;
  }
  return true;
}

/** The keys that act on the selection while no field takes them. */
function selectionKey(event: KeyboardEvent): boolean {
  const move = MOVES[event.key];
  if (move !== undefined) {
    send({ kind: "step", key: move, across: across() });
    return true;
  }
  if (event.key === "Enter") {
    send({ kind: "open", id: null });
    return true;
  }
  if ((event.key === "Backspace" || event.key === "Delete") && current !== null && current.selected.length > 0) {
    void confirmRemoval(current.selected);
    return true;
  }
  return false;
}

document.addEventListener("keydown", (event) => {
  if (event.isComposing || composing || document.querySelector("dialog[open]") !== null) {
    return;
  }
  const commanding = event.metaKey || event.ctrlKey;
  if (commanding && command(event)) {
    event.preventDefault();
    return;
  }
  if (event.target === search) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Enter") {
      event.preventDefault();
      if (event.key === "Enter") {
        send({ kind: "open", id: null });
      } else {
        send({ kind: "step", key: event.key === "ArrowDown" ? "down" : "up", across: across() });
      }
    } else if (event.key === "Escape" && search.value !== "") {
      event.preventDefault();
      search.value = "";
      send({ kind: "search", text: "" });
    }
    return;
  }
  if (typing(event.target) || menu.open) {
    return;
  }
  if (selectionKey(event)) {
    event.preventDefault();
  } else if (event.key.length === 1 && !commanding && !event.altKey) {
    // Typing anywhere searches, as the launcher always did.
    search.focus();
  }
});

search.addEventListener("input", () => {
  if (!composing) {
    send({ kind: "search", text: search.value });
  }
});
search.addEventListener("compositionstart", () => {
  composing = true;
});
search.addEventListener("compositionend", () => {
  composing = false;
  send({ kind: "search", text: search.value });
});

grouping.addEventListener("change", () => {
  const chosen = grouping.value;
  if (chosen === "none" || chosen === "site" || chosen === "collection" || chosen === "added") {
    send({ kind: "group", grouping: chosen });
  }
});
sort.addEventListener("change", () => {
  const chosen = sort.value;
  if (chosen === "newest" || chosen === "oldest" || chosen === "title" || chosen === "link" || chosen === "opened" || chosen === "manual") {
    send({ kind: "sort", sort: chosen });
  }
});

addButton.append(icon("plus"), "Add");
addButton.addEventListener("click", () => openAdd(null));
settingsButton.append(icon("gear"));
newFolderButton.append(icon("folder-plus"));
newFolderButton.addEventListener("click", () => void newFolder(current?.here ?? null));
settingsButton.addEventListener("click", () => openSettings());
find("search-glyph", HTMLElement).append(icon("search"));
find("sort-glyph", HTMLElement).append(icon("sort"));

for (const close of settings.querySelectorAll("[data-close]")) {
  close.addEventListener("click", () => settings.close());
}
for (const tab of settings.querySelectorAll<HTMLButtonElement>(".tab")) {
  const name = tab.dataset["tab"] ?? "";
  tab.append(icon(name === "theme" ? "contrast" : "sliders"), name === "theme" ? "Theme" : "General");
  tab.addEventListener("click", () => {
    for (const other of settings.querySelectorAll<HTMLButtonElement>(".tab")) {
      other.setAttribute("aria-selected", String(other === tab));
    }
    for (const pane of settings.querySelectorAll<HTMLElement>(".pane")) {
      pane.hidden = pane.dataset["pane"] !== name;
    }
    void drawThemes();
  });
}

/** Imports a bookmark file of any kind a browser exports or keeps. */
function importBookmarks(file: File): void {
  void file.arrayBuffer().then((buffer) => send({ kind: "import", bytes: Array.from(new Uint8Array(buffer)) }));
}

importHtml.addEventListener("click", () => importFile.click());
importFile.addEventListener("change", () => {
  const file = importFile.files?.[0];
  importFile.value = "";
  if (file !== undefined) {
    importBookmarks(file);
  }
});

/** The link a drop from a browser carries, from its list of links or its text. */
function droppedLink(transfer: DataTransfer): string | null {
  const listed = transfer
    .getData("text/uri-list")
    .split(/\r?\n/u)
    .find((line) => line !== "" && !line.startsWith("#"));
  const link = (listed ?? transfer.getData("text/plain")).trim();
  return link === "" ? null : link;
}

/** Whether a drag brings a file from outside the window, which the window imports as bookmarks rather than opening it in place of itself. */
function bringsFile(event: DragEvent): boolean {
  return event.dataTransfer?.types.includes("Files") ?? false;
}

/** Whether a drag brings a link from outside the window. */
function bringsLink(event: DragEvent): boolean {
  const types = event.dataTransfer?.types ?? [];
  return (
    !bringsFile(event) &&
    !types.includes(DRAGGED) &&
    !types.includes(DRAGGED_FOLDERS) &&
    (types.includes("text/uri-list") || types.includes("text/plain"))
  );
}

/** What the hint over the window says while a link or a file hovers over it. */
const DROP_LINK = "Drop to add the link";
const DROP_FILE = "Drop to import the bookmarks";

document.addEventListener("dragover", (event) => {
  const file = bringsFile(event);
  if (file || bringsLink(event)) {
    event.preventDefault();
    dropHint.textContent = file ? DROP_FILE : DROP_LINK;
    dropHint.hidden = false;
  }
});
document.addEventListener("dragleave", (event) => {
  if (event.relatedTarget === null) {
    dropHint.hidden = true;
  }
});
document.addEventListener("drop", (event) => {
  dropHint.hidden = true;
  const file = event.dataTransfer?.files[0];
  if (bringsFile(event)) {
    event.preventDefault();
    if (file !== undefined) {
      importBookmarks(file);
    }
  } else if (bringsLink(event) && event.dataTransfer !== null) {
    event.preventDefault();
    openAdd(droppedLink(event.dataTransfer));
  }
});

// The webview's own menu offers to reload or inspect the page, which an app has no use for; a field keeps its editing menu.
document.addEventListener("contextmenu", (event) => {
  if (typing(event.target)) {
    return;
  }
  event.preventDefault();
  if (event.target === content || event.target === content.firstElementChild) {
    menu.show(event, [
      { label: "New Link…", act: () => openAdd(null) },
      { label: "New Collection…", act: () => void newFolder(current?.here ?? null) },
    ]);
  }
});

/** Carries out a command a person chose from the app's menu. */
function menuCommand(id: string): void {
  if (id === "settings") {
    openSettings();
  } else if (id === "new-link") {
    openAdd(null);
  } else if (id === "new-collection") {
    void newFolder(current?.here ?? null);
  } else if (id === "import") {
    importFile.click();
  } else if (id === "cards" || id === "list" || id === "columns") {
    send({ kind: "layout", layout: id });
  }
}
content.addEventListener("click", (event) => {
  if (event.target === content) {
    content.focus();
  }
});
content.addEventListener("dragstart", (event) => {
  const dragged = event.target instanceof HTMLElement ? event.target.dataset["id"] : undefined;
  if (dragged !== undefined && current !== null && !current.selected.includes(dragged)) {
    send({ kind: "select", id: dragged, extend: extend(event) });
  }
});

window.addEventListener("focus", () => send({ kind: "show" }));
void listen(SUMMONED, () => {
  search.focus();
  search.select();
});
void listen(SYNCED, refresh);
void listen(ICONED, refresh);
void listenText(COMMAND, menuCommand);

send({ kind: "show" });

export {};
