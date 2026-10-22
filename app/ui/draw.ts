// Draws the view the library answers with: the sidebar, the toolbar, and the shortcuts as cards, a list, or columns.
// Every text reaches the page as text; the handlers the page passes in carry out what a person does.
// The cards, the list, and each column draw only the lines in sight, at heights the geometry counts, from the rows the page holds.

import { button, el, icon } from "./dom.js";
import { Frame, Geometry, sections } from "./rows.js";
import type { Line, Metrics, Rows, Section } from "./rows.js";
import type {
  CollectionCount,
  ColumnEntry,
  ColumnName,
  ColumnView,
  Extend,
  DragMaterial,
  Item,
  Layout,
  ListingView,
  RowView,
  ScopeName,
  Sort,
  Target,
  View,
} from "./types.js";
import { inline, thumbnail } from "./url.js";

/** The kind of a drag that carries shortcuts, and folders among them, inside the window. */
export const DRAGGED = "application/x-youarel-target";

/** What a person does to the drawn view. */
export interface Handlers {
  prepareDrag(target: Target): Promise<DragMaterial>;
  endDrag(token: string): void;
  hover(token: string, folder: string | null): Promise<number | null>;
  spring(token: string, folder: string): void;
  failed(error: unknown): void;
  scope(scope: ScopeName): void;
  /** Selects the row at a place, as a click with its modifiers asks. */
  select(id: string, place: number, extend: Extend): void;
  open(id: string): void;
  /** Opens the menu of the shortcut at a place, which acts on the selection when the selection takes it. */
  menu(event: MouseEvent, item: Item, place: number, selected: boolean): void;
  collectionMenu(event: MouseEvent, id: string, name: string, mark: string): void;
  /** Opens the menu of the trash in the sidebar, which empties it. */
  trashMenu(event: MouseEvent): void;
  newCollection(): void;
  /** Files the shortcuts and folders dropped on a collection, or on no collection. */
  dropOnCollection(target: Target, collection: string | null): void;
  /** Moves the shortcuts and folders dropped on the trash into it. */
  trash(target: Target): void;
  /** Puts shortcuts and folders in the trash back where they were. */
  putBack(target: Target): void;
  retitle(id: string, title: string): void;
  renameFolder(id: string, name: string): void;
  /** Saves one field of a shortcut that a person typed over in place. */
  saveField(item: Item, field: "alias" | "note", value: string): void;
  column(depth: number, entry: ColumnEntry, place: number, extend: Extend): void;
  copy(link: string): void;
  edit(item: Item): void;
  sort(sort: Sort): void;
  /** Opens the menu of the list's columns, which shows or hides each. */
  columnMenu(event: MouseEvent): void;
  /** Opens or closes a folder of the list's outline in place. */
  disclose(id: string, open: boolean, deep: boolean): void;
  /** Opens or closes a collection of the sidebar's tree. */
  discloseSidebar(id: string, open: boolean): void;
  layout(layout: Layout): void;
  icon(image: HTMLImageElement, name: string): void;
  /** Asks for the entries of the column at a depth from one place to before another, which a scroll of that column shows. */
  wantEntries(depth: number, first: number, end: number): void;
}

const LAYOUTS: readonly (readonly [Layout, string, string])[] = [
  ["cards", "Cards", "cards"],
  ["list", "List", "list"],
  ["columns", "Columns", "columns"],
];

const DATE = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const DATE_WITH_YEAR = new Intl.DateTimeFormat("en", { year: "numeric", month: "short", day: "numeric" });
const MOMENT = new Intl.DateTimeFormat("en", { year: "numeric", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
const NUMBER = new Intl.NumberFormat("en");
const HALVES = 2;

/** The height of a row of the list, which the style sheet draws and the geometry counts. */
const ROW_HEIGHT = 32;
/** The height of a heading of the list. */
const LIST_HEADING_HEIGHT = 40;
/** The height of a card. */
const CARD_HEIGHT = 224;
/** The height of a card's thumbnail. */
const THUMBNAIL_HEIGHT = 132;
/** The gap between two cards, across and down. */
const CARD_GAP = 16;
/** The narrowest a card grows. */
const CARD_MIN_WIDTH = 216;
/** The room at either side of the cards, and above them. */
const CARDS_INSET = 16;
const CARDS_TOP = 4;
/** The height of a heading of the cards. */
const CARD_HEADING_HEIGHT = 44;
/** The height of an entry of a column, and of a heading there. */
const ENTRY_HEIGHT = 28;
const COLUMN_HEADING_HEIGHT = 30;
/** How far past the band in sight the page draws lines, so a short scroll shows rows at once. */
const OVERSCAN = 480;

/** Sets the heights the style sheet draws with from the ones the geometry counts with, so both hold the same numbers. */
export function applyMetrics(): void {
  const root = document.documentElement.style;
  const sizes: readonly (readonly [string, number])[] = [
    ["--row-height", ROW_HEIGHT],
    ["--list-heading-height", LIST_HEADING_HEIGHT],
    ["--card-height", CARD_HEIGHT],
    ["--thumbnail-height", THUMBNAIL_HEIGHT],
    ["--card-gap", CARD_GAP],
    ["--cards-inset", CARDS_INSET],
    ["--cards-top", CARDS_TOP],
    ["--card-heading-height", CARD_HEADING_HEIGHT],
    ["--entry-height", ENTRY_HEIGHT],
    ["--column-heading-height", COLUMN_HEADING_HEIGHT],
  ];
  for (const [name, size] of sizes) {
    root.setProperty(name, `${size}px`);
  }
}

/** A day's date, with its year when it is not this one. */
export function day(time: number): string {
  const date = new Date(time);
  return date.getFullYear() === new Date().getFullYear() ? DATE.format(date) : DATE_WITH_YEAR.format(date);
}

/** A moment with its time of day. */
export function moment(time: number): string {
  return MOMENT.format(new Date(time));
}

/** A count of shortcuts in words. */
export function shortcuts(count: number): string {
  return count === 1 ? "1 shortcut" : `${NUMBER.format(count)} shortcuts`;
}

/** A count of folders in words. */
export function folders(count: number): string {
  return count === 1 ? "1 folder" : `${NUMBER.format(count)} folders`;
}

/** How many folders and shortcuts, in words. */
export function described(count: number, folderCount: number): string {
  const links = count - folderCount;
  if (folderCount === 0) {
    return shortcuts(links);
  }
  return links === 0 ? folders(folderCount) : `${folders(folderCount)} and ${shortcuts(links)}`;
}

function sameScope(left: ScopeName, right: ScopeName): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

/** Whether a value read from a drag is a target of a move. */
function isTarget(value: unknown): value is Target {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const fields = value as Record<string, unknown>;
  const ids = fields["ids"];
  return fields["kind"] === "selection"
    || (fields["kind"] === "drag" && typeof fields["token"] === "string")
    || (fields["kind"] === "ids" && Array.isArray(ids) && ids.every((id) => typeof id === "string"));
}

/** The text a drag carries under `kind`, or null. */
function carriedText(event: DragEvent, kind: string): string | null {
  const data = event.dataTransfer?.getData(kind) ?? "";
  return data === "" ? null : data;
}

/** What a drag of shortcuts carries, from its data. */
function carriedTarget(event: DragEvent): Target | null {
  const data = carriedText(event, DRAGGED);
  if (data === null) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(data);
    return isTarget(parsed) ? parsed : null;
  } catch (_invalidJson: unknown) {
    return null;
  }
}

/** Whether a drag carries shortcuts or folders from inside the window. */
function carriesEntries(event: DragEvent): boolean {
  const types = event.dataTransfer?.types ?? [];
  return types.includes(DRAGGED);
}

/** Makes an entry hand the shortcuts or the folders dropped on it to `take`, showing what the drop does while they hover. */
function dropTarget(entry: HTMLElement, label: string, take: (target: Target) => void): void {
  const hint = el("span", "drop-label", label);
  entry.addEventListener("dragover", (event) => {
    if (event.dataTransfer !== null && carriesEntries(event)) {
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = "move";
      entry.classList.add("target");
      if (!hint.isConnected) {
        entry.append(hint);
      }
    }
  });
  entry.addEventListener("dragleave", () => {
    entry.classList.remove("target");
    hint.remove();
  });
  entry.addEventListener("drop", (event) => {
    entry.classList.remove("target");
    hint.remove();
    const target = carriedTarget(event);
    if (target !== null) {
      event.preventDefault();
      event.stopPropagation();
    }
    if (target !== null) {
      take(target);
    }
  });
}

/** Makes an entry file the shortcuts or the folders dropped on it into `collection`, or on no collection. */
function filing(entry: HTMLElement, label: string, collection: string | null, on: Handlers): void {
  dropTarget(entry, label, (target) => on.dropOnCollection(target, collection));
  if (collection !== null) { springLoaded(entry, collection, on); }
}

let activeDrag: DragMaterial | null = null;
let stopSpring: (() => void) | null = null;

function springLoaded(entry: HTMLElement, folder: string, on: Handlers): void {
  let pending: { readonly token: string } | null = null;
  let timer: number | undefined;
  const stop = (): void => {
    const stopped = pending;
    pending = null;
    window.clearTimeout(timer);
    if (stopped !== null) { void on.hover(stopped.token, null).catch(on.failed); }
    if (stopSpring === stop) { stopSpring = null; }
  };
  entry.addEventListener("dragover", (event) => {
    if (!carriesEntries(event) || activeDrag === null || pending !== null) { return; }
    stopSpring?.();
    const asked = { token: activeDrag.token };
    pending = asked;
    stopSpring = stop;
    void on.hover(asked.token, folder).then((delay) => {
      if (pending !== asked || delay === null) { return; }
      timer = window.setTimeout(() => {
        if (pending === asked && entry.isConnected) { on.spring(asked.token, folder); }
      }, delay);
    }).catch(on.failed);
  });
  entry.addEventListener("dragleave", (event) => {
    if (!(event.relatedTarget instanceof Node) || !entry.contains(event.relatedTarget)) { stop(); }
  });
  entry.addEventListener("drop", stop);
}

function draggable(element: HTMLElement, target: Target, on: Handlers): void {
  let ready: DragMaterial | null = null;
  let request: object | null = null;
  element.draggable = true;
  element.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || !element.draggable) { return; }
    const asked = {};
    request = asked;
    ready = null;
    void on.prepareDrag(target).then((prepared) => {
      if (request === asked) { ready = prepared; }
      else { on.endDrag(prepared.token); }
    }).catch(on.failed);
  });
  element.addEventListener("dragstart", (event) => {
    if (!element.draggable) { return; }
    if (ready === null || event.dataTransfer === null) {
      event.preventDefault();
      on.failed("The selection is still preparing to drag. Try again.");
      return;
    }
    activeDrag = ready;
    event.dataTransfer.setData(DRAGGED, JSON.stringify({ kind: "drag", token: ready.token }));
    if (ready.links !== "") {
      event.dataTransfer.setData("text/uri-list", ready.links);
      event.dataTransfer.setData("text/plain", ready.links);
    }
    event.dataTransfer.effectAllowed = "copyMove";
    if (ready.count > 1) {
      const badge = el("div", "drag-badge", `${NUMBER.format(ready.count)} items`);
      document.body.append(badge);
      const bounds = badge.getBoundingClientRect();
      event.dataTransfer.setDragImage(badge, bounds.width / HALVES, bounds.height / HALVES);
      window.requestAnimationFrame(() => badge.remove());
    }
  });
  element.addEventListener("dragend", () => {
    stopSpring?.();
    request = null;
    if (ready !== null) { on.endDrag(ready.token); }
    if (activeDrag?.token === ready?.token) { activeDrag = null; }
    ready = null;
  });
}

/** Whether the window stands in the trash, whose rows neither move nor take a name typed over them. */
export function inTrash(view: View): boolean {
  return view.sidebar.scope.kind === "trash";
}

/** The clicks of a double click, which every second click of a run completes. */
const CLICK_PAIR = 2;
/** The longest gap between the two clicks of a double click, in milliseconds. */
const DOUBLE_CLICK_MS = 500;
/** The last click on a text that a double click types over, by what it names, since the first click may draw that text anew. */
let lastClick = { name: "", at: Number.NEGATIVE_INFINITY };
/** The text a person types over now, by what it names, and what they typed so far, which a redraw opens again. */
let typing: { readonly name: string; draft: string } | null = null;

/**
 * Lets a person double-click `target` to type over `value` in place; `name` tells the texts apart across redraws, such as `title:` and an identifier.
 *
 * Return saves, Escape gives the old text back, and leaving the field saves too; a note takes several lines, which Command or Control with Return saves.
 */
function editable(target: HTMLElement, name: string, value: string, commit: (value: string) => void, lines = false): void {
  target.dataset["editable"] = name;
  target.addEventListener("youarel-rename", () => begin(value));
  target.addEventListener("click", (event) => {
    const second = lastClick.name === name && event.timeStamp - lastClick.at < DOUBLE_CLICK_MS;
    lastClick = second ? { name: "", at: Number.NEGATIVE_INFINITY } : { name, at: event.timeStamp };
    if (second) {
      event.stopPropagation();
      begin(value);
    }
  });
  target.addEventListener("dblclick", (event) => {
    event.preventDefault();
    event.stopPropagation();
    begin(value);
  });
  // A redraw that lands while the person types draws the field again with what they typed.
  if (typing?.name === name) {
    begin(typing.draft);
  }
  function begin(start: string): void {
    if (target.querySelector(".in-place") !== null) {
      return;
    }
    const field = lines ? el("textarea", "in-place") : el("input", "in-place");
    field.value = start;
    const editing = { name, draft: start };
    typing = editing;
    const shown = [...target.childNodes];
    const holder = target.closest<HTMLElement>("[draggable]");
    const dragged = holder?.draggable ?? false;
    if (holder !== null) {
      holder.draggable = false;
    }
    let done = false;
    const finish = (save: boolean): void => {
      if (done) {
        return;
      }
      done = true;
      if (typing === editing) {
        typing = null;
      }
      if (save && field.value.trim() !== value.trim()) {
        commit(field.value);
        return;
      }
      if (holder !== null) {
        holder.draggable = dragged;
      }
      target.replaceChildren(...shown);
    };
    const box: HTMLElement = field;
    box.addEventListener("click", (click) => click.stopPropagation());
    box.addEventListener("dblclick", (click) => click.stopPropagation());
    box.addEventListener("keydown", (key) => {
      key.stopPropagation();
      if (key.key === "Escape") {
        key.preventDefault();
        finish(false);
      } else if (key.key === "Enter" && !key.isComposing && (!lines || key.metaKey || key.ctrlKey)) {
        key.preventDefault();
        finish(true);
      }
    });
    box.addEventListener("input", () => {
      editing.draft = field.value;
    });
    // A field that a redraw takes away loses the focus without the person leaving it, and the redraw opens it again.
    box.addEventListener("blur", () => {
      if (field.isConnected) {
        finish(true);
      }
    });
    target.replaceChildren(field);
    field.focus();
    if (start === value) {
      field.select();
    }
  }
}

/** Starts editing the selected entry's first editable name. */
export function rename(id: string): boolean {
  const target = document.querySelector(`[data-id="${CSS.escape(id)}"] [data-editable]`);
  if (target === null) { return false; }
  target.dispatchEvent(new Event("youarel-rename"));
  return true;
}

/** A folder's glyph, or the mark a person gave it in its place. */
function folderGlyph(mark: string): Element {
  return mark === "" ? icon("folder") : el("span", "mark", mark);
}

/** Lets a folder be dragged onto another folder. */
function draggableFolder(element: HTMLElement, target: Target, on: Handlers): void {
  draggable(element, target, on);
}

/** An entry that acts as a button yet can hold a field typed in place, which a button cannot. */
function pressable(className: string, ...children: (Node | string)[]): HTMLElement {
  const made = el("div", className);
  made.setAttribute("role", "button");
  made.tabIndex = 0;
  made.append(...children);
  made.addEventListener("keydown", (event) => {
    if (event.target === made && (event.key === " " || (event.key === "Enter" && made.closest("#content") === null))) {
      event.preventDefault();
      made.click();
    }
  });
  return made;
}

function scopeButton(label: string, glyph: string, count: number, current: boolean): HTMLElement {
  const entry = pressable("scope", icon(glyph), el("span", "label", label), el("span", "count", NUMBER.format(count)));
  entry.setAttribute("aria-current", String(current));
  return entry;
}

/** The sidebar: the fixed scopes and the collections, with their counts. */
export function sidebar(nav: HTMLElement, view: View, on: Handlers): void {
  const { sidebar: side } = view;
  const fixed: readonly (readonly [ScopeName, string, string, number])[] = [
    [{ kind: "all" }, "All", "all", side.all],
    [{ kind: "unsorted" }, "Unsorted", "inbox", side.unsorted],
    [{ kind: "recent" }, "Recently added", "clock", side.recent],
  ];
  const entries: HTMLElement[] = fixed.map(([scope, label, glyph, count]) => {
    const entry = scopeButton(label, glyph, count, sameScope(scope, side.scope));
    entry.addEventListener("click", () => on.scope(scope));
    if (scope.kind === "unsorted") {
      filing(entry, "Take out", null, on);
    }
    return entry;
  });
  const collectionsHead = el("div", "section");
  const add = el("button", "icon-button");
  add.type = "button";
  add.title = "New collection";
  add.setAttribute("aria-label", "New collection");
  add.append(icon("plus"));
  add.addEventListener("click", () => on.newCollection());
  collectionsHead.append(el("span", "", "Collections"), add);
  entries.push(collectionsHead);
  for (const collection of side.collections) {
    const scope: ScopeName = { kind: "collection", id: collection.id };
    const current = sameScope(scope, side.scope);
    const entry = scopeButton(collection.name, "folder", collection.count, current);
    if (collection.mark !== "") {
      entry.querySelector(":scope > .glyph")?.replaceWith(folderGlyph(collection.mark));
    }
    // The tree sets each folder in by its depth, and a folder that holds folders shows a triangle that opens or closes it.
    entry.style.setProperty("--depth", String(collection.depth));
    const toggle = el("button", "disclosure");
    toggle.type = "button";
    if (collection.opens) {
      toggle.setAttribute("aria-expanded", String(collection.open));
      toggle.setAttribute("aria-label", collection.open ? `Close ${collection.name}` : `Open ${collection.name}`);
      toggle.append(icon("chevron"));
      toggle.addEventListener("click", (event) => {
        event.stopPropagation();
        on.discloseSidebar(collection.id, !collection.open);
      });
      toggle.addEventListener("dblclick", (event) => event.stopPropagation());
    } else {
      toggle.disabled = true;
      toggle.tabIndex = -1;
    }
    entry.prepend(toggle);
    // A click on the collection shown already draws nothing new, so the second click of a double click finds the name it types over.
    entry.addEventListener("click", () => {
      if (!current) {
        on.scope(scope);
      }
    });
    entry.addEventListener("contextmenu", (event) => on.collectionMenu(event, collection.id, collection.name, collection.mark));
    filing(entry, "Move here", collection.id, on);
    draggableFolder(entry, { kind: "ids", ids: [collection.id] }, on);
    const label = entry.querySelector<HTMLElement>(".label");
    if (label !== null) {
      editable(label, `sidebar:${collection.id}`, collection.name, (name) => on.renameFolder(collection.id, name));
    }
    entries.push(entry);
  }
  const trash: ScopeName = { kind: "trash" };
  const bin = scopeButton("Trash", "trash", side.trash, sameScope(trash, side.scope));
  bin.classList.add("trash-scope");
  bin.addEventListener("click", () => on.scope(trash));
  bin.addEventListener("contextmenu", (event) => on.trashMenu(event));
  dropTarget(bin, "Move to Trash", (target) => on.trash(target));
  entries.push(bin);
  nav.replaceChildren(...entries);
}

/** The layout switch, as a group of three buttons. */
export function layouts(group: HTMLElement, chosen: Layout, choose: (layout: Layout) => void): void {
  group.replaceChildren(
    ...LAYOUTS.map(([layout, label, glyph]) => {
      const choice = button("", "", glyph);
      choice.title = label;
      choice.setAttribute("role", "radio");
      choice.setAttribute("aria-label", label);
      choice.setAttribute("aria-checked", String(layout === chosen));
      choice.addEventListener("click", () => choose(layout));
      return choice;
    }),
  );
}

/** The hues of the color wheel, which a site's tile takes one of. */
const HUES = 360;
/** The factor of the string hash that spreads the sites over the hues. */
const HASH_FACTOR = 31;

/** The hue of a site's tile, the same for the site wherever it shows. */
function hue(site: string): number {
  let hash = 0;
  for (const character of site) {
    hash = (hash * HASH_FACTOR + (character.codePointAt(0) ?? 0)) % HUES;
  }
  return hash;
}

/** An icon of a shortcut, or a tile with its site's first letter in its site's color, so shortcuts without icons still tell apart. */
function favicon(name: string | null, site: string, on: Handlers): HTMLElement {
  if (name === null) {
    const [first] = site;
    if (first === undefined) {
      const blank = el("span", "favicon blank");
      blank.append(icon("link"));
      return blank;
    }
    const tile = el("span", "favicon tile", first.toUpperCase());
    tile.style.setProperty("--tile-hue", String(hue(site)));
    return tile;
  }
  const image = el("img", "favicon");
  image.alt = "";
  // A drag that starts on the icon moves the shortcut, as one that starts beside it does, rather than the picture.
  image.draggable = false;
  on.icon(image, name);
  return image;
}

/** How a click with modifiers changes the selection. */
export function extend(event: MouseEvent): Extend {
  if (event.shiftKey) {
    return "range";
  }
  return event.metaKey || event.ctrlKey ? "toggle" : "replace";
}

/** Where a row stands in the listing: its place, and whether the selection takes it. */
interface Spot {
  readonly place: number;
  readonly selected: boolean;
  readonly cut: boolean;
}

/** Marks an element of a row with what the window reads back of it: its identifier, its place, and whether the selection takes it. */
function marked(element: HTMLElement, id: string, spot: Spot): void {
  element.dataset["id"] = id;
  element.dataset["place"] = String(spot.place);
  element.setAttribute("aria-selected", String(spot.selected));
  element.classList.toggle("cut", spot.cut);
}

/** Selects a row on a click, except the one row selected already, which a second click types over. */
function selecting(element: HTMLElement, id: string, spot: Spot, view: View, on: Handlers): void {
  element.addEventListener("click", (event) => {
    const how = extend(event);
    const alone = view.selection.count === 1 && spot.selected;
    if (how !== "replace" || !alone) {
      on.select(id, spot.place, how);
    }
  });
}

/** Wires what a person does to one shortcut: select, open, drag outside the trash, and the menu. */
function interactive(element: HTMLElement, item: Item, spot: Spot, view: View, on: Handlers): void {
  marked(element, item.id, spot);
  if (!inTrash(view)) { draggable(element, spot.selected ? { kind: "selection" } : { kind: "ids", ids: [item.id] }, on); }
  selecting(element, item.id, spot, view, on);
  element.addEventListener("dblclick", () => on.open(item.id));
  element.addEventListener("contextmenu", (event) => on.menu(event, item, spot.place, spot.selected));
}

function card(item: Item, spot: Spot, view: View, on: Handlers): HTMLElement {
  const made = el("article", "card");
  const frame = el("div", "thumbnail");
  frame.append(thumbnail(item.url), favicon(item.icon, item.url.name, on));
  const meta = el("div", "meta");
  const time = el("time", "", day(item.added));
  time.dateTime = new Date(item.added).toISOString();
  meta.append(el("span", "collection", item.collection ?? "No collection"), time);
  const title = el("div", "title", item.title);
  if (!inTrash(view)) {
    editable(title, `card:${item.id}`, item.title, (typed) => on.retitle(item.id, typed));
  }
  made.append(frame, title, meta);
  interactive(made, item, spot, view, on);
  return made;
}

/** Wires what a person does to one folder of the cards or the list: select, open it in the window, drag it, drop on it, and its menu. */
function folderInteractive(element: HTMLElement, folder: CollectionCount, spot: Spot, view: View, on: Handlers): void {
  marked(element, folder.id, spot);
  // A folder the outline opens in place opens or closes on every second click of a run, since the system counts a run of quick clicks and sends the double click only after the second.
  const inPlace = view.listing.outline;
  element.addEventListener("click", (event) => {
    if (inPlace && event.detail >= CLICK_PAIR && event.detail % CLICK_PAIR === 0) {
      on.open(folder.id);
      return;
    }
    const how = extend(event);
    const alone = view.selection.count === 1 && spot.selected;
    if (how !== "replace" || !alone) {
      on.select(folder.id, spot.place, how);
    }
  });
  if (!inPlace) {
    element.addEventListener("dblclick", () => on.open(folder.id));
  }
  element.addEventListener("contextmenu", (event) => on.collectionMenu(event, folder.id, folder.name, folder.mark));
  // A folder in the trash takes no drop and moves only out of the trash through its menu.
  if (!inTrash(view)) {
    filing(element, "Move here", folder.id, on);
    draggableFolder(element, spot.selected ? { kind: "selection" } : { kind: "ids", ids: [folder.id] }, on);
  }
}

/** What a folder's card says below its name: how many shortcuts it holds, and in the trash the folder it came from. */
function folderMeta(folder: CollectionCount, origin: string | null): string {
  return origin === null ? shortcuts(folder.count) : `${shortcuts(folder.count)} from ${origin}`;
}

function folderCard(folder: CollectionCount, origin: string | null, spot: Spot, view: View, on: Handlers): HTMLElement {
  const made = el("article", "card folder-card");
  const frame = el("div", "thumbnail");
  frame.append(folderGlyph(folder.mark));
  const meta = el("div", "meta");
  meta.append(el("span", "collection", folderMeta(folder, origin)));
  made.append(frame, el("div", "title", folder.name), meta);
  folderInteractive(made, folder, spot, view, on);
  return made;
}

function empty(view: View): HTMLElement {
  return el("p", "empty", view.search === "" ? "Nothing here yet." : "Nothing matches the search.");
}

/** The heading of the folders, which sets them apart from the shortcuts after them. */
const FOLDERS = "Folders";
/** The heading of the shortcuts after the folders, when no grouping heads them. */
const SHORTCUTS = "Shortcuts";

/** The sections of the listing: the folders, then the shortcuts under their headings, and under the headings of the folders and the shortcuts on the cards when both show and nothing else heads them. */
function listingSections(listing: ListingView, named: boolean): Section[] {
  if (listing.outline) {
    return sections(listing.total, []);
  }
  const links = listing.total - listing.folders;
  const both = named && listing.folders > 0 && links > 0;
  const marks: Section[] = [];
  if (listing.folders > 0) {
    marks.push({ start: 0, count: listing.folders, label: both ? FOLDERS : null });
  }
  if (listing.groups.length > 0) {
    marks.push(...listing.groups.map((group) => ({ start: group.place, count: group.count, label: group.label })));
  } else {
    marks.push({ start: listing.folders, count: links, label: both ? SHORTCUTS : null });
  }
  return sections(listing.total, marks);
}

/** A heading the shortcuts after it fall under, with how many they are. */
function groupHeading(label: string, count: number): HTMLElement {
  const heading = el("h3", "group-heading", label);
  heading.append(el("span", "count", NUMBER.format(count)));
  return heading;
}

/** Sets each element at the height of its line in a frame, for the band a scroll of the frame shows. */
function place(elements: HTMLCollection, lines: readonly Line[], frame: Frame, band: Seen): void {
  lines.forEach((line, index) => {
    const element = elements[index];
    if (element instanceof HTMLElement) {
      element.style.setProperty("--top", `${frame.place(line.top, band.own, band.client)}px`);
    }
  });
}

/** The places of a listing's rows: from the first to past the last. */
export interface Span {
  readonly first: number;
  readonly end: number;
}

/** A listing drawn in the content: the element that holds it, its geometry, and how a scroll of the content shows its lines. */
export interface Drawn {
  readonly element: HTMLElement;
  readonly geometry: Geometry;
  /** Draws the lines in sight for a scroll of the content from the rows held, and answers the places of the rows in sight. */
  show(scroll: number, client: number): Span;
  /** The places of the rows in sight for a scroll of the content. */
  span(scroll: number, client: number): Span;
  /** The scroll of the content that brings the line of a place into sight, nearest the scroll it has. */
  reveal(placed: number, scroll: number, client: number): number;
  anchor(placed: number, offset: number, client: number): number;
}

/** What a scroll of a frame shows: the frame's own scroll, the height of the client, and the height of the lines at the band's top. */
interface Seen {
  readonly own: number;
  readonly client: number;
  readonly top: number;
}

function anchored(frame: Frame, inset: number, line: { readonly top: number; readonly offset: number }, client: number): number {
  const top = line.top - line.offset;
  return inset + (top < 0 ? top : frame.scroll(top, client));
}

/** The band of a frame that a scroll of the content shows, for a frame set an offset below the content's top. */
function seen(frame: Frame, offset: number, scroll: number, client: number): Seen {
  const own = Math.max(0, scroll - offset);
  return { own, client, top: frame.top(own, client) };
}

/** The scroll of the content that brings a line of a frame into sight, nearest the scroll it has. */
function revealed(frame: Frame, offset: number, line: { readonly top: number; readonly height: number }, band: Seen): number {
  const top =
    line.top < band.top
      ? line.top
      : line.top + line.height > band.top + band.client
        ? line.top + line.height - band.client
        : band.top;
  return offset + frame.scroll(top, band.client);
}

/** The metrics of the cards in a content of a width: as many across as fit the narrowest card, as the style sheet's grid fits them. */
export function cardMetrics(width: number): Metrics {
  const room = Math.max(0, width - CARDS_INSET - CARDS_INSET);
  const across = Math.max(1, Math.floor((room + CARD_GAP) / (CARD_MIN_WIDTH + CARD_GAP)));
  return { across, pitch: CARD_HEIGHT + CARD_GAP, heading: CARD_HEADING_HEIGHT };
}

/** A line of cards whose rows the page does not hold yet. */
function placeholderCard(): HTMLElement {
  return el("article", "card placeholder");
}

/** The card of one row of the listing. */
function rowCard(row: RowView | undefined, place: number, view: View, on: Handlers): HTMLElement {
  if (row === undefined || row.kind === "gone") {
    return placeholderCard();
  }
  const spot = { place, selected: row.selected, cut: row.cut };
  return row.kind === "folder" ? folderCard(row.folder, row.origin, spot, view, on) : card(row.item, spot, view, on);
}

/** Draws the lines of a band again only when they or the rows held changed since the last drawing. */
class Band {
  private drawn = "";

  changed(lines: readonly { readonly top: number }[], version: number): boolean {
    const signature = `${lines[0]?.top ?? -1}:${lines.at(-1)?.top ?? -1}:${lines.length}:${version}`;
    const changed = signature !== this.drawn;
    this.drawn = signature;
    return changed;
  }
}

/** The folders the window stands in, then the shortcuts, as cards, each part under its heading when both show. */
export function cards(view: View, held: Rows<RowView>, on: Handlers, width: number): Drawn | HTMLElement {
  if (view.listing.total === 0) {
    return empty(view);
  }
  const geometry = new Geometry(listingSections(view.listing, true), cardMetrics(width));
  const frame = new Frame(geometry.height);
  const holder = el("div", "cards");
  holder.style.setProperty("--frame-height", `${frame.height}px`);
  holder.style.setProperty("--across", String(geometry.metrics.across));
  const band = new Band();
  return {
    element: holder,
    geometry,
    show(scroll, client) {
      const sight = seen(frame, CARDS_TOP, scroll, client);
      const lines = geometry.lines(sight.top - OVERSCAN, sight.top + client + OVERSCAN);
      if (band.changed(lines, held.version)) {
        holder.replaceChildren(
          ...lines.map((line) => {
            if (line.kind === "heading") {
              return groupHeading(line.section.label ?? "", line.section.count);
            }
            const shown = el("div", "card-line");
            for (let at = line.first; at < line.first + line.count; at += 1) {
              shown.append(rowCard(held.get(at), at, view, on));
            }
            return shown;
          }),
        );
      }
      place(holder.children, lines, frame, sight);
      return geometry.span(sight.top, sight.top + client);
    },
    span(scroll, client) {
      const sight = seen(frame, CARDS_TOP, scroll, client);
      return geometry.span(sight.top, sight.top + client);
    },
    reveal(placed, scroll, client) {
      const line = { top: geometry.top(placed), height: geometry.metrics.pitch };
      return revealed(frame, CARDS_TOP, line, seen(frame, CARDS_TOP, scroll, client));
    },
    anchor(placed, offset, client) {
      return anchored(frame, CARDS_TOP, { top: geometry.top(placed), offset }, client);
    },
  };
}

/** The heading of each column of the list after the name. */
export const COLUMN_TITLES: Readonly<Record<ColumnName, string>> = { link: "URL", collection: "Collection", added: "Added" };
/** The orders each column's heading sorts by: the first, then the second when it sorts by the first already. */
const COLUMN_SORTS: Readonly<Record<ColumnName, readonly Sort[]>> = {
  link: ["link"],
  collection: [],
  added: ["newest", "oldest"],
};
/** The orders the name's heading sorts by. */
const NAME_SORTS: readonly Sort[] = ["title"];

/** A column's heading, the whole of which sorts by it. */
function header(label: string, view: View, sorts: readonly Sort[], on: Handlers): HTMLTableCellElement {
  const cell = el("th");
  const sorting = el("button");
  sorting.type = "button";
  sorting.textContent = label;
  const [first, second] = sorts;
  if (first === undefined) {
    cell.textContent = label;
    return cell;
  }
  if (sorts.includes(view.sort)) {
    cell.setAttribute("aria-sort", view.sort === "oldest" ? "ascending" : "descending");
    sorting.append(icon("sort"));
  }
  sorting.addEventListener("click", () => on.sort(view.sort === first && second !== undefined ? second : first));
  cell.append(sorting);
  return cell;
}

/** The cell of a shortcut's column. */
function shortcutCell(item: Item, column: ColumnName): HTMLTableCellElement {
  if (column === "link") {
    const cell = el("td");
    cell.append(inline(item.url));
    return cell;
  }
  if (column === "collection") {
    return el("td", "muted", item.collection ?? "—");
  }
  const cell = el("td");
  cell.append(el("time", "", day(item.added)));
  return cell;
}

/** The name cell of a row, set in by its depth in the outline. */
function nameCell(depth: number, ...children: Node[]): HTMLTableCellElement {
  const cell = el("td");
  const inner = el("div", "name-cell");
  inner.style.setProperty("--depth", String(depth));
  inner.append(...children);
  cell.append(inner);
  return cell;
}

function row(item: Item, depth: number, spot: Spot, view: View, on: Handlers): HTMLTableRowElement {
  const line = el("tr");
  const title = el("span", "title", item.title);
  if (!inTrash(view)) {
    editable(title, `row:${item.id}`, item.title, (typed) => on.retitle(item.id, typed));
  }
  // A shortcut in the outline keeps the room of a folder's disclosure, so the names line up.
  const spacer = view.listing.outline ? [el("span", "disclosure")] : [];
  line.append(nameCell(depth, ...spacer, favicon(item.icon, item.url.name, on), title));
  line.append(...view.listColumns.map((column) => shortcutCell(item, column)));
  interactive(line, item, spot, view, on);
  return line;
}

/** A folder's disclosure: a triangle that opens or closes the folder in place, as an explorer's list does. */
function disclosure(folder: CollectionCount, open: boolean, on: Handlers): HTMLElement {
  const toggle = el("button", "disclosure");
  toggle.type = "button";
  toggle.setAttribute("aria-expanded", String(open));
  toggle.setAttribute("aria-label", open ? `Close ${folder.name}` : `Open ${folder.name}`);
  toggle.append(icon("chevron"));
  toggle.addEventListener("click", (event) => {
    event.stopPropagation();
    on.disclose(folder.id, !open, event.altKey);
  });
  toggle.addEventListener("dblclick", (event) => event.stopPropagation());
  return toggle;
}

/** The facts of a folder's row: none, but the folder it came from in the column of the collection while it waits in the trash. */
function folderCell(column: ColumnName, origin: string | null): HTMLTableCellElement {
  return column === "collection" && origin !== null ? el("td", "muted", origin) : el("td");
}

function folderRow(
  folder: CollectionCount,
  origin: string | null,
  depth: number,
  open: boolean | null,
  spot: Spot,
  view: View,
  on: Handlers,
): HTMLTableRowElement {
  const line = el("tr", "folder-row");
  const lead = open === null ? [] : [disclosure(folder, open, on)];
  line.append(nameCell(depth, ...lead, folderGlyph(folder.mark), el("span", "title", folder.name), el("span", "count", NUMBER.format(folder.count))));
  line.append(...view.listColumns.map((column) => folderCell(column, origin)));
  folderInteractive(line, folder, spot, view, on);
  return line;
}

/** A row of the list whose entry the page does not hold yet. */
function placeholderRow(cells: number): HTMLTableRowElement {
  const line = el("tr", "placeholder");
  const cell = el("td");
  cell.colSpan = cells;
  line.append(cell);
  return line;
}

/** The row of one place of the listing. */
function listRow(shown: RowView | undefined, place: number, cells: number, view: View, on: Handlers): HTMLTableRowElement {
  if (shown === undefined || shown.kind === "gone") {
    return placeholderRow(cells);
  }
  const spot = { place, selected: shown.selected, cut: shown.cut };
  return shown.kind === "folder"
    ? folderRow(shown.folder, shown.origin, shown.depth, shown.open, spot, view, on)
    : row(shown.item, shown.depth, spot, view, on);
}

/** A row of the list that stands for the rows before or after the band in sight, at a height set as the band moves. */
function spacer(cells: number): HTMLTableRowElement {
  const line = el("tr", "spacer");
  const cell = el("td");
  cell.colSpan = cells;
  line.append(cell);
  return line;
}

/** The height of one line of the list. */
function lineHeight(line: Line): number {
  return line.kind === "heading" ? LIST_HEADING_HEIGHT : ROW_HEIGHT;
}

/** The folders the window stands in, then the shortcuts, as a table that sorts by its headings and opens folders in place. */
export function list(view: View, held: Rows<RowView>, on: Handlers): Drawn | HTMLElement {
  if (view.listing.total === 0) {
    return empty(view);
  }
  const geometry = new Geometry(listingSections(view.listing, false), { across: 1, pitch: ROW_HEIGHT, heading: LIST_HEADING_HEIGHT });
  const table = el("table", "list");
  const columns = el("colgroup");
  columns.append(el("col", "name"), ...view.listColumns.map((column) => el("col", column)));
  const head = el("thead");
  const headings = el("tr");
  headings.append(
    header("Name", view, NAME_SORTS, on),
    ...view.listColumns.map((column) => header(COLUMN_TITLES[column], view, COLUMN_SORTS[column], on)),
  );
  headings.addEventListener("contextmenu", (event) => on.columnMenu(event));
  head.append(headings);
  const body = el("tbody");
  table.append(columns, head, body);
  const cells = headings.childElementCount;
  const frame = new Frame(geometry.height);
  const before = spacer(cells);
  const after = spacer(cells);
  const band = new Band();
  return {
    element: table,
    geometry,
    show(scroll, client) {
      const sight = seen(frame, body.offsetTop, scroll, client);
      const lines = geometry.lines(sight.top - OVERSCAN, sight.top + client + OVERSCAN);
      if (band.changed(lines, held.version)) {
        const shown = lines.map((line) => {
          if (line.kind === "rows") {
            return listRow(held.get(line.first), line.first, cells, view, on);
          }
          const cell = el("td");
          cell.colSpan = cells;
          cell.append(groupHeading(line.section.label ?? "", line.section.count));
          const heading = el("tr", "group-row");
          heading.append(cell);
          return heading;
        });
        body.replaceChildren(before, ...shown, after);
      }
      // The spacers take the frame's height the lines in sight leave, so the lines sit where the scroll shows them.
      const first = lines[0];
      const above = first === undefined ? 0 : Math.max(0, frame.place(first.top, sight.own, client));
      const drawn = lines.reduce((sum, line) => sum + lineHeight(line), 0);
      before.style.setProperty("--height", `${above}px`);
      after.style.setProperty("--height", `${Math.max(0, frame.height - above - drawn)}px`);
      return geometry.span(sight.top, sight.top + client);
    },
    span(scroll, client) {
      const sight = seen(frame, body.offsetTop, scroll, client);
      return geometry.span(sight.top, sight.top + client);
    },
    reveal(placed, scroll, client) {
      const line = { top: geometry.top(placed), height: ROW_HEIGHT };
      return revealed(frame, body.offsetTop, line, seen(frame, body.offsetTop, scroll, client));
    },
    anchor(placed, offset, client) {
      return anchored(frame, body.offsetTop, { top: geometry.top(placed), offset }, client);
    },
  };
}

/** Wires what a person does to a folder of a column outside the trash: drop on it, drag it, and type over its name. */
function columnFolder(made: HTMLElement, label: HTMLElement, shown: ColumnEntry, on: Handlers): void {
  filing(made, "Move here", shown.id, on);
  draggableFolder(made, shown.selected ? { kind: "selection" } : { kind: "ids", ids: [shown.id] }, on);
  editable(label, `column-folder:${shown.id}`, shown.name, (name) => on.renameFolder(shown.id, name));
}

/** Wires what a person does to a shortcut of a column outside the trash: type over its name and drag it. */
function columnShortcut(made: HTMLElement, label: HTMLElement, shown: ColumnEntry, on: Handlers): void {
  editable(label, `column:${shown.id}`, shown.name, (title) => on.retitle(shown.id, title));
  draggable(made, shown.selected ? { kind: "selection" } : { kind: "ids", ids: [shown.id] }, on);
}

function entry(shown: ColumnEntry, depth: number, place: number, trail: boolean, trashed: boolean, on: Handlers): HTMLElement {
  const label = el("span", "label", shown.name);
  const made = pressable(trail ? "entry trail" : "entry");
  marked(made, shown.id, { place, selected: shown.selected, cut: shown.cut });
  if (shown.kind === "gone") {
    return made;
  }
  if (shown.kind === "folder") {
    made.append(folderGlyph(shown.mark), label);
    if (shown.opens) {
      made.append(icon("chevron"));
    }
    made.addEventListener("contextmenu", (event) => on.collectionMenu(event, shown.id, shown.name, shown.mark));
    if (!trashed) {
      columnFolder(made, label, shown, on);
    }
  } else {
    made.append(favicon(shown.icon, shown.site, on), label);
    // A double click on the name types over it; one elsewhere on the entry opens the link.
    made.addEventListener("dblclick", () => on.open(shown.id));
    if (!trashed) {
      columnShortcut(made, label, shown, on);
    }
  }
  // The deepest selection drawn already takes no click, so a double click can type over its name; one on the trail closes the columns after it.
  made.addEventListener("click", (event) => {
    if (!shown.selected || trail || extend(event) !== "replace") {
      on.column(depth, shown, place, extend(event));
    }
  });
  return made;
}

function fact(list: HTMLElement, term: string, value: Node | string): HTMLElement {
  const description = el("dd");
  description.append(value);
  list.append(el("dt", "", term), description);
  return description;
}

/** The preview of the selected shortcut: its link as type, its title, what opens it, and its facts. */
function preview(item: Item, on: Handlers): HTMLElement {
  const pane = el("section", "preview");
  const frame = el("div", "thumbnail");
  frame.append(thumbnail(item.url));
  const buttons = el("div", "buttons");
  const open = button("Open", "go", "open");
  open.addEventListener("click", () => on.open(item.id));
  const copy = button("Copy URL", "secondary", "copy");
  copy.addEventListener("click", () => on.copy(item.link));
  const edit = button("Edit…", "secondary");
  edit.addEventListener("click", () => on.edit(item));
  buttons.append(open, copy, edit);
  const facts = el("dl", "facts");
  fact(facts, "Collection", item.collection ?? "No collection");
  const alias = fact(facts, "Alias", item.alias === "" ? "None" : item.alias);
  editable(alias, `alias:${item.id}`, item.alias, (typed) => on.saveField(item, "alias", typed));
  const note = fact(facts, "Note", item.note === "" ? "None" : item.note);
  editable(note, `note:${item.id}`, item.note, (typed) => on.saveField(item, "note", typed), true);
  fact(facts, "Added", moment(item.added));
  fact(facts, "Last opened", item.opened === null ? "Never" : moment(item.opened));
  const link = el("span", "link");
  link.append(inline(item.url));
  const title = el("h2", "", item.title);
  editable(title, `preview:${item.id}`, item.title, (typed) => on.retitle(item.id, typed));
  pane.append(frame, title, link, buttons, facts);
  return pane;
}

/** The preview of a shortcut that waits in the trash: its link, its title, the folder it came from, and what puts it back. */
function trashedPreview(item: Item, on: Handlers): HTMLElement {
  const pane = el("section", "preview");
  const frame = el("div", "thumbnail");
  frame.append(thumbnail(item.url));
  const buttons = el("div", "buttons");
  const back = button("Put Back", "go");
  back.addEventListener("click", () => on.putBack({ kind: "ids", ids: [item.id] }));
  const copy = button("Copy URL", "secondary", "copy");
  copy.addEventListener("click", () => on.copy(item.link));
  buttons.append(back, copy);
  const facts = el("dl", "facts");
  fact(facts, "From", item.collection ?? "No collection");
  fact(facts, "Added", moment(item.added));
  const link = el("span", "link");
  link.append(inline(item.url));
  pane.append(frame, el("h2", "", item.title), link, buttons, facts);
  return pane;
}

/** A column of the walk drawn: its scrolled element, its geometry, and how it draws the entries in sight. */
export interface DrawnColumn {
  readonly element: HTMLElement;
  readonly geometry: Geometry;
  /** Draws the entries in sight from the entries held, and asks for the ones the page lacks. */
  show(): void;
  /** Scrolls the column until the entry at a place shows. */
  reveal(placed: number): void;
  anchor(placed: number, offset: number): void;
}

/** Where a column of the walk stands: its depth, whether its selection marks the trail to a deeper column, and whether it lists the trash. */
interface ColumnSpot {
  readonly depth: number;
  readonly trail: boolean;
  readonly trashed: boolean;
}

/** One column of the walk, which draws only the entries in sight and asks for more as a person scrolls it. */
function column(shown: ColumnView, spot: ColumnSpot, held: Rows<ColumnEntry>, on: Handlers): DrawnColumn {
  const { depth, trail, trashed } = spot;
  const marks = shown.groups.map((group) => ({ start: group.place, count: group.count, label: group.label }));
  const geometry = new Geometry(sections(shown.total, marks), { across: 1, pitch: ENTRY_HEIGHT, heading: COLUMN_HEADING_HEIGHT });
  const frame = new Frame(geometry.height);
  const scroller = el("div", "column");
  const body = el("div", "column-body");
  body.style.setProperty("--frame-height", `${frame.height}px`);
  scroller.append(shown.total === 0 ? el("p", "none", "Empty") : body);
  const band = new Band();
  const show = (): void => {
    const sight = seen(frame, body.offsetTop, scroller.scrollTop, scroller.clientHeight);
    const lines = geometry.lines(sight.top - OVERSCAN, sight.top + sight.client + OVERSCAN);
    if (band.changed(lines, held.version)) {
      body.replaceChildren(
        ...lines.map((line) => {
          if (line.kind === "heading") {
            return el("h3", "column-heading", line.section.label ?? "");
          }
          const listed = held.get(line.first);
          return listed === undefined
            ? el("div", "entry placeholder")
            : entry(listed, depth, line.first, trail && shown.selected === line.first, trashed, on);
        }),
      );
    }
    place(body.children, lines, frame, sight);
    const span = geometry.span(sight.top, sight.top + sight.client);
    on.wantEntries(depth, span.first, span.end);
  };
  const reveal = (placed: number): void => {
    const sight = seen(frame, body.offsetTop, scroller.scrollTop, scroller.clientHeight);
    const line = { top: geometry.top(placed), height: ENTRY_HEIGHT };
    scroller.scrollTop = revealed(frame, body.offsetTop, line, sight);
  };
  let pending = false;
  scroller.addEventListener("scroll", () => {
    if (!pending) {
      pending = true;
      window.requestAnimationFrame(() => {
        pending = false;
        show();
      });
    }
  });
  const anchor = (placed: number, offset: number): void => {
    scroller.scrollTop = anchored(frame, body.offsetTop, { top: geometry.top(placed), offset }, scroller.clientHeight);
  };
  return { element: scroller, geometry, show, reveal, anchor };
}

/** The columns of the folder walk, and the preview of the shortcut it ends on. */
export function columns(view: View, held: readonly Rows<ColumnEntry>[], on: Handlers): { readonly element: HTMLElement; readonly columns: readonly DrawnColumn[] } {
  const walk = el("div", "columns");
  // The selection of the deepest column is the one the keys move; the ones before it mark the trail that leads there.
  let deepest = -1;
  view.columns.forEach((shown, depth) => {
    if (shown.selected !== null) {
      deepest = depth;
    }
  });
  const trashed = inTrash(view);
  const drawn = view.columns.flatMap((shown, depth) => {
    const rows = held[depth];
    return rows === undefined ? [] : [column(shown, { depth, trail: depth < deepest, trashed }, rows, on)];
  });
  walk.append(...drawn.map((shown) => shown.element));
  if (view.preview !== null) {
    walk.append(trashed ? trashedPreview(view.preview, on) : preview(view.preview, on));
  }
  return { element: walk, columns: drawn };
}
