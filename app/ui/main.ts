// The window's side of the library: it sends what a person does to the library and draws the view it gets back.
// It decides nothing the library decides; every shortcut's text reaches the page as text, never as markup.
// The page holds the rows around the place in sight and asks the library for the others as a person scrolls, so a listing of millions draws as fast as one of a few.

import {
  COLUMN_TITLES,
  DRAGGED,
  applyMetrics,
  cardMetrics,
  cards,
  columns,
  described,
  inTrash,
  layouts,
  list,
  rename,
  shortcuts,
  sidebar,
} from "./draw.js";
import type { Drawn, DrawnColumn, Handlers } from "./draw.js";
import { button, el, find, icon } from "./dom.js";
import { Rows } from "./rows.js";
import { AddSheet, AskSheet, EditSheet, MarkSheet, Menu, Picker } from "./sheets.js";
import type { MenuChoice } from "./sheets.js";
import { drawSync } from "./sync.js";
import { DevicesPane } from "./devices.js";
import { themePane, wear } from "./theme.js";
import { invoke, listen, listenProgress, listenText } from "./types.js";
import type {
  Browser,
  Choice,
  ColumnEntry,
  ColumnName,
  EmojiGroup,
  EntriesView,
  DragMaterial,
  Intent,
  Item,
  Key,
  Layout,
  Progress,
  RowView,
  RowsView,
  Sort,
  Target,
  ThemeOption,
  View,
} from "./types.js";

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
/** The event the library raises with how far a move or a removal came. */
const PROGRESS = "progress";
/** How many rows the page asks for beyond those in sight, before and after them. */
const MARGIN = 100;
/** The most rows one call for a page asks for, as the library answers at most. */
const MOST_ROWS = 1000;
/** How long a scroll rests before the page tells the library where it stands. */
const REST_MS = 150;
/** How many collections a picker shows at a time. */
const PICKED = 200;

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
const exportHtml = find("export-html", HTMLButtonElement);
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
/** The rows the page holds of the listing the cards or the list show, and of each column. */
const listingRows = new Rows<RowView>();
let columnRows: Rows<ColumnEntry>[] = [];
/** The listing drawn in the content, and the columns. */
let drawnListing: Drawn | null = null;
let drawnColumns: readonly DrawnColumn[] = [];
/** How many views the page drew, so a page that answers an older one changes nothing. */
let drawnViews = 0;
/** Whether a call for rows is in flight, whether another band waits for one, and the last call, which goes once for a view. */
let fetching = false;
let wanting = false;
let lastCall = "";
/** Whether a view that a changed store asks for waits in the queue already. */
let refreshing = false;
let restTimer: number | undefined;

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

/** Shows how far a move or a removal came, until the view after it arrives. */
function showProgress(progress: Progress): void {
  window.clearTimeout(noticeTimer);
  notice.textContent = progress.text;
  notice.classList.remove("error");
  notice.hidden = false;
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
  void devicesPane.refresh();
  if (current !== null) {
    openAs.value = current.opening.layout;
    void openAt.open(current.opening.start);
  }
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
    made.addEventListener("click", () =>
      send({ kind: "column", depth: step.depth, entry: { kind: "folder", id: step.id }, place: null, extend: "replace" }),
    );
    return [el("span", "separator", TRAIL_SEPARATOR), made];
  });
  trailNav.replaceChildren(root, ...steps);
}

/** Draws the lines of the listing in sight, and asks for the rows the page lacks. */
function showListing(): void {
  if (drawnListing === null) {
    return;
  }
  const span = drawnListing.show(content.scrollTop, content.clientHeight);
  if (!listingRows.holds(span.first, span.end)) {
    void fetchRows(span.first, span.end);
  }
  void fetchIcons();
}

/** Asks the library for the rows around a span in sight, one call at a time, and draws them once they arrive; the same call for the same view goes once. */
async function fetchRows(first: number, end: number): Promise<void> {
  if (fetching) {
    wanting = true;
    return;
  }
  const asked = drawnViews;
  const start = Math.max(0, first - MARGIN);
  const limit = Math.min(MOST_ROWS, end + MARGIN - start);
  const call = `${asked}:${start}:${limit}`;
  if (call === lastCall) {
    return;
  }
  lastCall = call;
  fetching = true;
  try {
    const page = await invoke<RowsView>("page", { first: start, limit });
    if (asked === drawnViews) {
      if (page.key === listingRows.key) {
        listingRows.take(page.total, page.first, page.rows);
      } else {
        // The store changed since the view, so the whole view is drawn again.
        refresh();
      }
    }
  } catch (error: unknown) {
    notice.textContent = String(error);
    notice.classList.add("error");
    notice.hidden = false;
  } finally {
    fetching = false;
  }
  if (wanting) {
    wanting = false;
  }
  showListing();
}

/** Asks the library for the entries of a column around a span in sight, and draws them once they arrive. */
async function fetchEntries(depth: number, first: number, end: number): Promise<void> {
  const held = columnRows[depth];
  if (held === undefined || held.holds(first, end)) {
    return;
  }
  const asked = drawnViews;
  const start = Math.max(0, first - MARGIN);
  const limit = Math.min(MOST_ROWS, end + MARGIN - start);
  const page = await invoke<EntriesView>("column_page", { depth, first: start, limit });
  if (asked !== drawnViews) {
    return;
  }
  if (page.key !== held.key) {
    refresh();
    return;
  }
  held.take(page.total, page.first, page.entries);
  drawnColumns[depth]?.show();
  void fetchIcons();
}

/** Tells the library where the page stands once a scroll rests, so the next view answers around it, and holds the rows there anew. */
function rest(): void {
  window.clearTimeout(restTimer);
  restTimer = window.setTimeout(() => {
    if (drawnListing !== null) {
      const span = drawnListing.span(content.scrollTop, content.clientHeight);
      void fetchRows(span.first, span.end);
    }
  }, REST_MS);
}

interface ViewportPin {
  readonly identity: string;
  readonly column: number | null;
  readonly scroll: number;
  readonly anchor: { readonly id: string; readonly place: number; readonly offset: number } | null;
}

function pinViewport(scroller: HTMLElement, identity: string, column: number | null): ViewportPin {
  const area = scroller.getBoundingClientRect();
  const top = area.top + scroller.clientTop;
  const covered = Math.max(top, scroller.querySelector("thead")?.getBoundingClientRect().bottom ?? top);
  for (const entry of scroller.querySelectorAll<HTMLElement>("[data-id][data-place]")) {
    const id = entry.dataset["id"];
    const place = Number(entry.dataset["place"]);
    const bounds = entry.getBoundingClientRect();
    if (id !== undefined && Number.isSafeInteger(place) && place >= 0 && bounds.bottom > covered
      && bounds.top < top + scroller.clientHeight && bounds.right > area.left && bounds.left < area.right) {
      return { identity, column, scroll: scroller.scrollTop, anchor: { id, place, offset: bounds.top - top } };
    }
  }
  return { identity, column, scroll: scroller.scrollTop, anchor: null };
}

function viewportPins(): readonly ViewportPin[] {
  if (current === null) { return []; }
  if (current.layout !== "columns") { return [pinViewport(content, current.listing.identity, null)]; }
  return current.columns.flatMap((column, depth) => {
    const drawn = drawnColumns[depth];
    return drawn === undefined ? [] : [pinViewport(drawn.element, column.identity, depth)];
  });
}

/** Draws the cards, the list, or the columns of a view, each from the rows the page holds. */
function drawContent(view: View, answers: boolean, pins: readonly ViewportPin[] = []): void {
  const left = content.scrollLeft;
  drawnListing = null;
  drawnColumns = [];
  if (view.layout === "columns") {
    const walk = columns(view, columnRows, handlers);
    content.replaceChildren(walk.element);
    drawnColumns = walk.columns;
    view.columns.forEach((shown, depth) => {
      const drawn = walk.columns[depth];
      const pin = pins.find((pin) => pin.column === depth && pin.identity === shown.identity);
      if (!answers && drawn !== undefined && pin !== undefined) {
        drawn.element.scrollTop = pin.scroll;
        if (pin.anchor !== null) {
          const place = shown.anchor?.id === pin.anchor.id ? shown.anchor.place : pin.anchor.place;
          drawn.anchor(place, pin.anchor.offset);
        }
      }
      if (answers && drawn !== undefined && shown.selected !== null) {
        drawn.reveal(shown.selected);
      }
      drawn?.show();
    });
    if (answers) {
      // The deepest column and its preview stay at the right edge as the walk goes deeper or turns back up, as an explorer's columns do.
      content.scrollLeft = content.scrollWidth;
    } else {
      content.scrollLeft = left;
    }
    return;
  }
  const drawn = view.layout === "cards" ? cards(view, listingRows, handlers, content.clientWidth) : list(view, listingRows, handlers);
  if (drawn instanceof HTMLElement) {
    content.replaceChildren(drawn);
    return;
  }
  content.replaceChildren(drawn.element);
  drawnListing = drawn;
  if (view.listing.fresh) {
    content.scrollTop = 0;
  }
  const pin = pins.find((pin) => pin.column === null && pin.identity === view.listing.identity);
  if (!answers && pin !== undefined) {
    content.scrollTop = pin.scroll;
    if (pin.anchor !== null) {
      const place = view.listing.anchor?.id === pin.anchor.id ? view.listing.anchor.place : pin.anchor.place;
      content.scrollTop = drawn.anchor(place, pin.anchor.offset, content.clientHeight);
    }
  }
  const reveal = view.listing.reveal;
  if (answers && reveal !== null) {
    content.scrollTop = drawn.reveal(reveal, content.scrollTop, content.clientHeight);
  }
  showListing();
}

/** Draws a view; one that answers no intent of the person's keeps the scroll and the notice on screen, unless the notice is new. */
function draw(view: View, answers = true, pins: readonly ViewportPin[] = []): void {
  const shown = current?.notice?.text;
  current = view;
  historyMenu();
  drawnViews += 1;
  refreshing = false;
  listingRows.reset(view.listing.key, view.listing.total, view.listing.first, view.listing.rows);
  columnRows = view.columns.map((column) => {
    const held = new Rows<ColumnEntry>();
    held.reset(column.key, column.total, column.first, column.entries);
    return held;
  });
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
  drawContent(view, answers, pins);
  if (answers || view.notice?.text !== shown) {
    showNotice(view);
  }
  void fetchIcons();
}

/** Shows a failure of a call to the library. */
function failed(error: unknown): void {
  notice.textContent = String(error);
  notice.classList.add("error");
  notice.hidden = false;
}

/** Names the next history steps while fields keep their own text editing. */
function historyMenu(): void {
  void invoke("history_menu", {
    undo: current?.undo ?? null,
    redo: current?.redo ?? null,
    field: typing(document.activeElement),
  }).catch(failed);
}

document.addEventListener("focusin", historyMenu);
document.addEventListener("focusout", () => queueMicrotask(historyMenu));

/** Sends an intent after every earlier one, and draws the view it answers with. */
function send(intent: Intent): void {
  queue = queue.then(async () => {
    try {
      draw(await invoke<View>("act", { intent }));
    } catch (error: unknown) {
      failed(error);
    }
  });
}

function libraryCommand(command: "copy_selection" | "paste" | "export_bookmarks"): void {
  queue = queue.then(async () => {
    try {
      draw(await invoke<View>(command));
    } catch (error: unknown) {
      failed(error);
    }
  });
}

/** Draws the view as it stands after every earlier intent, as when a sync began or ended. */
function refresh(): void {
  if (refreshing) {
    return;
  }
  refreshing = true;
  queue = queue.then(async () => {
    try {
      const pins = viewportPins();
      const anchors = pins.flatMap((pin) => pin.anchor === null ? [] : [{ identity: pin.identity, column: pin.column, id: pin.anchor.id }]);
      draw(await invoke<View>("view_anchored", { anchors }), false, pins);
    } catch (error: unknown) {
      refreshing = false;
      failed(error);
    }
  });
}

/** The focused shortcut, from the rows held or the preview. */
function focused(): Item | null {
  if (current === null || current.focus === null) {
    return null;
  }
  const place = current.listing.focus;
  const held = place === null ? undefined : listingRows.get(place);
  if (held?.kind === "shortcut" && held.item.id === current.focus) {
    return held.item;
  }
  return current.preview?.id === current.focus ? current.preview : null;
}

/** How many cards a row of the grid holds now. */
function across(): number {
  return current?.layout === "cards" ? (drawnListing?.geometry.metrics.across ?? 1) : 1;
}

async function copy(link: string): Promise<void> {
  await invoke("copy", { text: link });
  notice.textContent = "Copied the URL";
  notice.classList.remove("error");
  notice.hidden = false;
}

const ask = new AskSheet();
const devicesPane = new DevicesPane(ask, failed);
const marking = new MarkSheet((query) => invoke<EmojiGroup[]>("emoji", { query }));
const menu = new Menu();
/** The collections a picker finds for what a person typed. */
const pick = (query: string, first: number): Promise<Choice[]> => invoke<Choice[]>("choices", { query, first, limit: PICKED });

/** The layout the settings open the window with. */
const openAs = find("open-as", HTMLSelectElement);
/** Keeps the place and the layout the settings show as what the window opens with. */
function chooseOpening(): void {
  const layout = openAs.value;
  if (layout === "cards" || layout === "list" || layout === "columns") {
    send({ kind: "opening", layout, start: openAt.value });
  }
}
/** The collection the settings open the window at, or every shortcut. */
const openAt = new Picker(find("open-at-search", HTMLInputElement), find("open-at", HTMLSelectElement), pick, ALL, chooseOpening);
openAs.addEventListener("change", chooseOpening);

/** Whether the window stands in the trash now. */
function trashShown(): boolean {
  return current !== null && inTrash(current);
}

/** Asks before deleting shortcuts and folders for good, naming how many, and deletes them once the person agrees. */
async function confirmDeletion(target: Target, total: number, folderCount: number): Promise<void> {
  const answer = await ask.ask({
    title: total === 1 && folderCount === 0 ? "Delete this shortcut immediately?" : `Delete ${described(total, folderCount)} immediately?`,
    text:
      folderCount === 0
        ? "It goes for good on every device of the account, and no undo brings it back."
        : "The shortcuts and collections inside go with them, for good, on every device of the account.",
    action: "Delete",
    answer: null,
  });
  if (answer !== null) {
    send({ kind: "remove", target });
  }
}

/** Asks once before emptying the trash, and empties it once the person agrees. */
async function confirmEmptying(): Promise<void> {
  const held = current?.sidebar.trash ?? 0;
  const answer = await ask.ask({
    title: "Empty the Trash?",
    text: `${held === 1 ? "The item" : `The ${held} items`} in the Trash go for good on every device of the account, and no undo brings them back.`,
    action: "Empty Trash",
    answer: null,
  });
  if (answer !== null) {
    send({ kind: "emptyTrash" });
  }
}

/** Moves what the selection takes to the trash, or, in the trash, asks before deleting it for good. */
function deleteSelection(immediately: boolean): void {
  if (current === null || current.selection.count === 0) {
    return;
  }
  if (immediately || trashShown()) {
    void confirmDeletion({ kind: "selection" }, current.selection.count, current.selection.folders);
  } else {
    send({ kind: "trash", target: { kind: "selection" } });
  }
}

const adding = new AddSheet(
  {
    clipboard: () => invoke<string | null>("clipboard"),
    title: (link) => invoke<string | null>("title", { link }),
    add: async (link, name, collection) => {
      try {
        const view = await invoke<View>("add", { link, title: name, collection });
        draw(view);
        return view.notice?.error !== true;
      } catch (error: unknown) {
        failed(error);
        return false;
      }
    },
  },
  pick,
);

const editing = new EditSheet(
  (fields) => send({ kind: "edit", fields }),
  (id) => send({ kind: "trash", target: { kind: "ids", ids: [id] } }),
  pick,
);

/** The choices of the menu of what waits in the trash: put it back, or delete it for good once the person agrees. */
function trashChoices(target: Target, total: number, folderCount: number): MenuChoice[] {
  return [
    { label: "Put Back", act: () => send({ kind: "putBack", target }) },
    "line",
    { label: "Delete Immediately…", danger: true, act: () => void confirmDeletion(target, total, folderCount) },
  ];
}

/** Opens the add sheet over the collection the window stands in. */
function openAdd(dropped: string | null): void {
  if (current !== null) {
    void adding.open(current.here, dropped);
  }
}

function edit(item: Item): void {
  editing.open(item);
}

/** Asks for a name and makes a collection inside `parent`, or at the top for null. */
async function newFolder(parent: Choice | null): Promise<void> {
  const name = await ask.ask({
    title: parent === null ? "New collection" : `New collection in ${parent.path}`,
    text: "",
    action: "Create",
    answer: "",
  });
  if (name !== null) {
    send({ kind: "newCollection", name, parent: parent?.id ?? null });
  }
}

const handlers: Handlers = {
  prepareDrag(target: Target): Promise<DragMaterial> {
    const prepared = queue.then(() => invoke<DragMaterial>("prepare_drag", { target }));
    queue = prepared.then(() => undefined).catch(failed);
    return prepared;
  },
  endDrag: (token) => send({ kind: "endDrag", token }),
  hover: (token, folder) => {
    const hovered = queue.then(() => invoke<number | null>("hover", { token, folder }));
    queue = hovered.then(() => undefined).catch(failed);
    return hovered;
  },
  spring: (token, folder) => send({ kind: "spring", token, folder }),
  failed,
  scope: (scope) => send({ kind: "scope", scope }),
  select: (id, place, how) => send({ kind: "select", id, extend: how, place }),
  open: (id) => send({ kind: "open", id }),
  menu: (event, item, place, selected) => {
    if (!selected) {
      send({ kind: "select", id: item.id, extend: "replace", place });
    }
    const selection = current?.selection ?? { count: 1, folders: 0 };
    const [total, folderCount]: readonly [number, number] = selected ? [selection.count, selection.folders] : [1, 0];
    const target: Target = selected ? { kind: "selection" } : { kind: "ids", ids: [item.id] };
    if (trashShown()) {
      menu.show(event, trashChoices(target, total, folderCount));
      return;
    }
    const choices: MenuChoice[] = [
      { label: "Open", act: () => send({ kind: "open", id: item.id }) },
      { label: "Copy URL", act: () => void copy(item.link) },
      { label: "Show in Folder", act: () => send({ kind: "showInFolder", id: item.id }) },
      "line",
      { label: "Edit…", act: () => edit(item) },
      "line",
      { label: "Move to Trash", act: () => send({ kind: "trash", target }) },
    ];
    menu.show(event, choices);
  },
  collectionMenu: (event, id, name, mark) => {
    const target: Target = { kind: "ids", ids: [id] };
    if (trashShown()) {
      menu.show(event, trashChoices(target, 1, 1));
      return;
    }
    menu.show(event, [
      { label: "New collection inside…", act: () => void newFolder({ id, path: name }) },
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
      { label: "Move to Trash", act: () => send({ kind: "trash", target }) },
    ]);
  },
  trashMenu: (event) => {
    menu.show(event, [{ label: "Empty Trash…", danger: true, act: () => void confirmEmptying() }]);
  },
  newCollection: () => void newFolder(null),
  dropOnCollection: (target, collection) => send({ kind: "file", target, collection }),
  trash: (target) => send({ kind: "trash", target }),
  putBack: (target) => send({ kind: "putBack", target }),
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
  column: (depth, entry, place, extend) => {
    if (entry.kind !== "gone") {
      send({ kind: "column", depth, entry: { kind: entry.kind, id: entry.id }, place, extend });
    }
  },
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
  disclose: (id, open, deep) => send({ kind: "disclose", id, open, deep }),
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
  wantEntries: (depth, first, end) => void fetchEntries(depth, first, end).catch(failed),
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
  } else if (key === "o" && !typing(event.target)) {
    send({ kind: "open", id: null });
  } else if (key === "[") {
    send({ kind: "back" });
  } else if (key === "]") {
    send({ kind: "forward" });
  } else if (key === "r") {
    send({ kind: "showInFolder", id: null });
  } else if (key === "z" && !typing(event.target)) {
    send({ kind: event.shiftKey ? "redo" : "undo" });
  } else if (key === "y" && !typing(event.target)) {
    send({ kind: "redo" });
  } else if (key === "k" || key === "f") {
    search.focus();
    search.select();
  } else if (key === "n" && event.shiftKey) {
    void newFolder(current?.here ?? null);
  } else if (key === "n") {
    openAdd(null);
  } else if (key === ",") {
    openSettings();
  } else if (key === "w") {
    void invoke("close").catch(failed);
  } else if (key === "e") {
    const item = focused();
    if (item !== null) {
      edit(item);
    }
  } else if (key === "a" && !typing(event.target)) {
    send({ kind: "selectAll" });
  } else if (key === "c" && !typing(event.target)) {
    libraryCommand("copy_selection");
  } else if (key === "x" && !typing(event.target)) {
    send({ kind: "cut" });
  } else if (key === "v" && !typing(event.target)) {
    libraryCommand("paste");
  } else if ((key === "backspace" || key === "delete") && !typing(event.target)) {
    // The command key with delete moves the selection to the trash, and with the option key too deletes it for good, as an explorer's keys do.
    deleteSelection(event.altKey);
  } else {
    return false;
  }
  return true;
}

/** The keys that act on the selection while no field takes them. */
function selectionKey(event: KeyboardEvent): boolean {
  if (event.key === "Escape") {
    send({ kind: "cancel" });
    return true;
  }
  const move = MOVES[event.key];
  if (move !== undefined) {
    send({ kind: "step", key: move, across: across(), extend: event.shiftKey });
    return true;
  }
  if (event.key === "Enter") {
    if (current?.selection.count === 1 && current.focus !== null && !inTrash(current)) {
      rename(current.focus);
    } else {
      send({ kind: "open", id: null });
    }
    return true;
  }
  if ((event.key === "Backspace" || event.key === "Delete") && current !== null && current.selection.count > 0) {
    deleteSelection(false);
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
        send({ kind: "step", key: event.key === "ArrowDown" ? "down" : "up", across: across(), extend: event.shiftKey });
      }
    } else if (event.key === "Escape") {
      event.preventDefault();
      if (search.value !== "") {
        search.value = "";
        send({ kind: "search", text: "" });
      }
      send({ kind: "cancel" });
    }
    return;
  }
  if (typing(event.target) || menu.open) {
    return;
  }
  if (selectionKey(event)) {
    event.preventDefault();
  } else if (Array.from(event.key).length === 1 && !commanding && !event.altKey) {
    event.preventDefault();
    send({ kind: "type", key: event.key });
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
  if (chosen === "newest" || chosen === "oldest" || chosen === "title" || chosen === "link" || chosen === "opened") {
    send({ kind: "sort", sort: chosen });
  }
});

applyMetrics();
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
  tab.append(icon(name === "theme" ? "contrast" : "sliders"), name === "theme" ? "Theme" : name === "devices" ? "Devices" : "General");
  tab.addEventListener("click", () => {
    for (const other of settings.querySelectorAll<HTMLButtonElement>(".tab")) {
      other.setAttribute("aria-selected", String(other === tab));
    }
    for (const pane of settings.querySelectorAll<HTMLElement>(".pane")) {
      pane.hidden = pane.dataset["pane"] !== name;
    }
    void drawThemes();
    void devicesPane.refresh();
  });
}

/** Imports a bookmark file of any kind a browser exports or keeps. */
function importBookmarks(file: File): void {
  void file.arrayBuffer().then((buffer) => send({ kind: "import", bytes: Array.from(new Uint8Array(buffer)) }));
}

importHtml.addEventListener("click", () => importFile.click());
exportHtml.addEventListener("click", () => {
  settings.close();
  libraryCommand("export_bookmarks");
});
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
  if (id === "cut" || id === "copy" || id === "paste" || id === "select-all") {
    if (typing(document.activeElement)) {
      document.execCommand(id === "select-all" ? "selectAll" : id);
    } else if (id === "cut") {
      send({ kind: "cut" });
    } else if (id === "select-all") {
      send({ kind: "selectAll" });
    } else {
      libraryCommand(id === "copy" ? "copy_selection" : "paste");
    }
  } else if (id === "undo" || id === "redo") {
    if (typing(document.activeElement)) {
      document.execCommand(id);
    } else {
      send({ kind: id });
    }
  } else if (id === "settings") {
    openSettings();
  } else if (id === "new-link") {
    openAdd(null);
  } else if (id === "new-collection") {
    void newFolder(current?.here ?? null);
  } else if (id === "import") {
    importFile.click();
  } else if (id === "export") {
    libraryCommand("export_bookmarks");
  } else if (id === "open") {
    send({ kind: "open", id: null });
  } else if (id === "back" || id === "forward") {
    send({ kind: id });
  } else if (id === "show-in-folder") {
    send({ kind: "showInFolder", id: null });
  } else if (id === "cards" || id === "list" || id === "columns") {
    send({ kind: "layout", layout: id });
  }
}
content.addEventListener("click", (event) => {
  if (event.target === content) {
    content.focus();
  }
});

let scrolling = false;
content.addEventListener("scroll", () => {
  if (!scrolling) {
    scrolling = true;
    window.requestAnimationFrame(() => {
      scrolling = false;
      showListing();
    });
  }
  rest();
});
let resizing: number | undefined;
window.addEventListener("resize", () => {
  window.clearTimeout(resizing);
  resizing = window.setTimeout(() => {
    if (current !== null && current.layout === "cards" && drawnListing !== null) {
      if (cardMetrics(content.clientWidth).across !== drawnListing.geometry.metrics.across) {
        drawContent(current, false, viewportPins());
      }
    }
  }, REST_MS);
});

window.addEventListener("focus", refresh);
void listen(SUMMONED, () => {
  search.focus();
  search.select();
});
void listen(SYNCED, refresh);
void listen(ICONED, refresh);
void listenText(COMMAND, menuCommand);
void listenProgress(PROGRESS, showProgress);

send({ kind: "show" });

export {};
