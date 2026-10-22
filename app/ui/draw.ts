// Draws the view the library answers with: the sidebar, the toolbar, and the shortcuts as cards, a list, or columns.
// Every text reaches the page as text; the handlers the page passes in carry out what a person does.

import { button, el, icon } from "./dom.js";
import type { CollectionCount, ColumnEntry, ColumnName, Extend, Item, Layout, ScopeName, Sort, View } from "./types.js";
import { inline, thumbnail } from "./url.js";

/** The kind of a drag that carries shortcuts inside the window. */
export const DRAGGED = "application/x-youarel-ids";
/** The kind of a drag that carries folders inside the window. */
export const DRAGGED_FOLDERS = "application/x-youarel-folders";

/** What a person does to the drawn view. */
export interface Handlers {
  scope(scope: ScopeName): void;
  select(id: string, extend: Extend): void;
  open(id: string): void;
  menu(event: MouseEvent, item: Item): void;
  collectionMenu(event: MouseEvent, id: string, name: string, mark: string): void;
  newCollection(): void;
  dropOnCollection(ids: readonly string[], collection: string | null): void;
  moveFolders(ids: readonly string[], collection: string | null): void;
  retitle(id: string, title: string): void;
  renameFolder(id: string, name: string): void;
  /** Saves one field of a shortcut that a person typed over in place. */
  saveField(item: Item, field: "alias" | "note", value: string): void;
  column(depth: number, entry: ColumnEntry): void;
  copy(link: string): void;
  edit(item: Item): void;
  sort(sort: Sort): void;
  /** Opens the menu of the list's columns, which shows or hides each. */
  columnMenu(event: MouseEvent): void;
  /** Opens or closes a folder of the list's outline in place. */
  disclose(id: string, open: boolean): void;
  /** Opens or closes a collection of the sidebar's tree. */
  discloseSidebar(id: string, open: boolean): void;
  layout(layout: Layout): void;
  icon(image: HTMLImageElement, name: string): void;
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

function sameScope(left: ScopeName, right: ScopeName): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

/** The identifiers a drag carries under `kind`, from its data. */
function carried(event: DragEvent, kind: string): readonly string[] | null {
  const data = event.dataTransfer?.getData(kind) ?? "";
  if (data === "") {
    return null;
  }
  const parsed: unknown = JSON.parse(data);
  return Array.isArray(parsed) && parsed.every((id) => typeof id === "string") ? parsed : null;
}

/** Whether a drag carries shortcuts or folders from inside the window. */
function carriesEntries(event: DragEvent): boolean {
  const types = event.dataTransfer?.types ?? [];
  return types.includes(DRAGGED) || types.includes(DRAGGED_FOLDERS);
}

/** Makes an entry take the shortcuts or the folders dropped on it into `collection`, showing what the drop does while they hover. */
function dropTarget(entry: HTMLElement, label: string, collection: string | null, on: Handlers): void {
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
    const links = carried(event, DRAGGED);
    const folders = carried(event, DRAGGED_FOLDERS);
    if (links !== null || folders !== null) {
      event.preventDefault();
      event.stopPropagation();
    }
    if (links !== null) {
      on.dropOnCollection(links, collection);
    }
    if (folders !== null) {
      on.moveFolders(folders, collection);
    }
  });
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

/** A folder's glyph, or the mark a person gave it in its place. */
function folderGlyph(mark: string): Element {
  return mark === "" ? icon("folder") : el("span", "mark", mark);
}

/** Lets a folder be dragged onto another folder. */
function draggableFolder(element: HTMLElement, id: string): void {
  element.draggable = true;
  element.addEventListener("dragstart", (event) => {
    event.dataTransfer?.setData(DRAGGED_FOLDERS, JSON.stringify([id]));
    if (event.dataTransfer !== null) {
      event.dataTransfer.effectAllowed = "move";
    }
  });
}

/** An entry that acts as a button yet can hold a field typed in place, which a button cannot. */
function pressable(className: string, ...children: (Node | string)[]): HTMLElement {
  const made = el("div", className);
  made.setAttribute("role", "button");
  made.tabIndex = 0;
  made.append(...children);
  made.addEventListener("keydown", (event) => {
    if (event.target === made && (event.key === "Enter" || event.key === " ")) {
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
      dropTarget(entry, "Take out", null, on);
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
    dropTarget(entry, "Move here", collection.id, on);
    draggableFolder(entry, collection.id);
    const label = entry.querySelector<HTMLElement>(".label");
    if (label !== null) {
      editable(label, `sidebar:${collection.id}`, collection.name, (name) => on.renameFolder(collection.id, name));
    }
    entries.push(entry);
  }
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

/** Wires what a person does to one shortcut: select, open, drag, and the menu. */
function interactive(element: HTMLElement, item: Item, view: View, on: Handlers): void {
  element.dataset["id"] = item.id;
  element.setAttribute("aria-selected", String(view.selected.includes(item.id)));
  element.draggable = true;
  element.addEventListener("click", (event) => {
    const how = extend(event);
    // A click on the one shortcut selected already draws nothing new, so a double click can type over its title.
    const alone = view.selected.length === 1 && view.selected.includes(item.id);
    if (how !== "replace" || !alone) {
      on.select(item.id, how);
    }
  });
  element.addEventListener("dblclick", () => on.open(item.id));
  element.addEventListener("contextmenu", (event) => on.menu(event, item));
  element.addEventListener("dragstart", (event) => {
    const ids = view.selected.includes(item.id) ? view.selected : [item.id];
    event.dataTransfer?.setData(DRAGGED, JSON.stringify(ids));
    event.dataTransfer?.setData("text/uri-list", item.link);
    if (event.dataTransfer !== null) {
      event.dataTransfer.effectAllowed = "copyMove";
    }
  });
}

function card(item: Item, view: View, on: Handlers): HTMLElement {
  const made = el("article", "card");
  const frame = el("div", "thumbnail");
  frame.append(thumbnail(item.url), favicon(item.icon, item.url.name, on));
  const meta = el("div", "meta");
  const time = el("time", "", day(item.added));
  time.dateTime = new Date(item.added).toISOString();
  meta.append(el("span", "collection", item.collection ?? "No collection"), time);
  const title = el("div", "title", item.title);
  editable(title, `card:${item.id}`, item.title, (typed) => on.retitle(item.id, typed));
  made.append(frame, title, meta);
  interactive(made, item, view, on);
  return made;
}

/** Wires what a person does to one folder of the cards or the list: select, open it in the window, drag it, drop on it, and its menu. */
function folderInteractive(element: HTMLElement, folder: CollectionCount, view: View, on: Handlers): void {
  element.dataset["id"] = folder.id;
  element.setAttribute("aria-selected", String(view.selected.includes(folder.id)));
  // A folder the outline opens in place opens or closes on every second click of a run, since the system counts a run of quick clicks and sends the double click only after the second.
  const inPlace = view.outline.length > 0;
  element.addEventListener("click", (event) => {
    if (inPlace && event.detail >= CLICK_PAIR && event.detail % CLICK_PAIR === 0) {
      on.open(folder.id);
      return;
    }
    const how = extend(event);
    const alone = view.selected.length === 1 && view.selected.includes(folder.id);
    if (how !== "replace" || !alone) {
      on.select(folder.id, how);
    }
  });
  if (!inPlace) {
    element.addEventListener("dblclick", () => on.open(folder.id));
  }
  element.addEventListener("contextmenu", (event) => on.collectionMenu(event, folder.id, folder.name, folder.mark));
  dropTarget(element, "Move here", folder.id, on);
  draggableFolder(element, folder.id);
}

function folderCard(folder: CollectionCount, view: View, on: Handlers): HTMLElement {
  const made = el("article", "card folder-card");
  const frame = el("div", "thumbnail");
  frame.append(folderGlyph(folder.mark));
  const meta = el("div", "meta");
  meta.append(el("span", "collection", shortcuts(folder.count)));
  made.append(frame, el("div", "title", folder.name), meta);
  folderInteractive(made, folder, view, on);
  return made;
}

function empty(view: View): HTMLElement {
  return el("p", "empty", view.search === "" ? "Nothing here yet." : "Nothing matches the search.");
}

/** The heading of the folders, which sets them apart from the shortcuts after them. */
const FOLDERS = "Folders";
/** The heading of the shortcuts after the folders, when no grouping heads them. */
const SHORTCUTS = "Shortcuts";

/** The cards of the folders the window stands in, then of the shortcuts, each part under its heading when both show. */
export function cards(view: View, on: Handlers): HTMLElement {
  if (view.items.length === 0 && view.folders.length === 0) {
    return empty(view);
  }
  const grid = el("div", "cards");
  const both = view.folders.length > 0 && view.items.length > 0;
  if (both) {
    grid.append(groupHeading(FOLDERS, view.folders.length));
  }
  grid.append(...view.folders.map((folder) => folderCard(folder, view, on)));
  for (const [label, items] of gathered(view)) {
    if (label !== null || both) {
      grid.append(groupHeading(label ?? SHORTCUTS, items.length));
    }
    grid.append(...items.map((item) => card(item, view, on)));
  }
  return grid;
}

/** The items under each heading in turn, or all of them under none when the window gathers nothing. */
function gathered(view: View): (readonly [string | null, readonly Item[]])[] {
  if (view.groups.length === 0) {
    return [[null, view.items]];
  }
  let start = 0;
  return view.groups.map((group) => {
    const items = view.items.slice(start, start + group.count);
    start += group.count;
    return [group.label, items] as const;
  });
}

/** A heading the shortcuts after it fall under, with how many they are. */
function groupHeading(label: string, count: number): HTMLElement {
  const heading = el("h3", "group-heading", label);
  heading.append(el("span", "count", NUMBER.format(count)));
  return heading;
}

/** The heading of each column of the list after the name. */
export const COLUMN_TITLES: Readonly<Record<ColumnName, string>> = { link: "URL", collection: "Collection", added: "Added" };
/** The orders each column's heading sorts by: the first, then the second when it sorts by the first already. */
const COLUMN_SORTS: Readonly<Record<ColumnName, readonly Sort[]>> = {
  link: ["link"],
  collection: ["manual"],
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

function row(item: Item, depth: number, view: View, on: Handlers): HTMLTableRowElement {
  const line = el("tr");
  const title = el("span", "title", item.title);
  editable(title, `row:${item.id}`, item.title, (typed) => on.retitle(item.id, typed));
  // A shortcut in the outline keeps the room of a folder's disclosure, so the names line up.
  const spacer = view.outline.length > 0 ? [el("span", "disclosure")] : [];
  line.append(nameCell(depth, ...spacer, favicon(item.icon, item.url.name, on), title));
  line.append(...view.listColumns.map((column) => shortcutCell(item, column)));
  interactive(line, item, view, on);
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
    on.disclose(folder.id, !open);
  });
  toggle.addEventListener("dblclick", (event) => event.stopPropagation());
  return toggle;
}

function folderRow(folder: CollectionCount, depth: number, open: boolean | null, view: View, on: Handlers): HTMLTableRowElement {
  const line = el("tr", "folder-row");
  const lead = open === null ? [] : [disclosure(folder, open, on)];
  line.append(nameCell(depth, ...lead, folderGlyph(folder.mark), el("span", "title", folder.name), el("span", "count", NUMBER.format(folder.count))));
  line.append(...view.listColumns.map(() => el("td")));
  folderInteractive(line, folder, view, on);
  return line;
}

/** The rows of the outline, each folder with the rows it opens beneath it. */
function outlineRows(view: View, on: Handlers): HTMLTableRowElement[] {
  const items = new Map(view.items.map((item) => [item.id, item]));
  const folders = new Map(view.folders.map((folder) => [folder.id, folder]));
  return view.outline.flatMap((shown) => {
    if (shown.kind === "folder") {
      const folder = folders.get(shown.id);
      return folder === undefined ? [] : [folderRow(folder, shown.depth, shown.open, view, on)];
    }
    const item = items.get(shown.id);
    return item === undefined ? [] : [row(item, shown.depth, view, on)];
  });
}

/** The folders the window stands in, then the shortcuts, as a table that sorts by its headings and opens folders in place. */
export function list(view: View, on: Handlers): HTMLElement {
  if (view.items.length === 0 && view.folders.length === 0) {
    return empty(view);
  }
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
  if (view.outline.length > 0) {
    body.append(...outlineRows(view, on));
  } else {
    body.append(...view.folders.map((folder) => folderRow(folder, 0, null, view, on)));
    for (const [label, items] of gathered(view)) {
      if (label !== null) {
        const cell = el("td");
        cell.colSpan = headings.childElementCount;
        cell.append(groupHeading(label, items.length));
        const line = el("tr", "group-row");
        line.append(cell);
        body.append(line);
      }
      body.append(...items.map((item) => row(item, 0, view, on)));
    }
  }
  table.append(columns, head, body);
  return table;
}

function entry(shown: ColumnEntry, depth: number, trail: boolean, on: Handlers): HTMLElement {
  const label = el("span", "label", shown.name);
  const made = pressable(trail ? "entry trail" : "entry");
  made.setAttribute("aria-selected", String(shown.selected));
  if (shown.kind === "folder") {
    made.append(folderGlyph(shown.mark), label);
    if (shown.opens) {
      made.append(icon("chevron"));
    }
    made.addEventListener("contextmenu", (event) => on.collectionMenu(event, shown.id, shown.name, shown.mark));
    dropTarget(made, "Move here", shown.id, on);
    draggableFolder(made, shown.id);
    editable(label, `column-folder:${shown.id}`, shown.name, (name) => on.renameFolder(shown.id, name));
  } else {
    made.append(favicon(shown.icon, shown.site, on), label);
    // A double click on the name types over it; one elsewhere on the entry opens the link.
    made.addEventListener("dblclick", () => on.open(shown.id));
    editable(label, `column:${shown.id}`, shown.name, (title) => on.retitle(shown.id, title));
    made.draggable = true;
    made.addEventListener("dragstart", (event) => {
      event.dataTransfer?.setData(DRAGGED, JSON.stringify([shown.id]));
      if (event.dataTransfer !== null) {
        event.dataTransfer.effectAllowed = "copyMove";
      }
    });
  }
  // The deepest selection drawn already takes no click, so a double click can type over its name; one on the trail closes the columns after it.
  made.addEventListener("click", () => {
    if (!shown.selected || trail) {
      on.column(depth, shown);
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

/** The columns of the folder walk, and the preview of the shortcut it ends on. */
export function columns(view: View, on: Handlers): HTMLElement {
  const walk = el("div", "columns");
  // The selection of the deepest column is the one the keys move; the ones before it mark the trail that leads there.
  let deepest = -1;
  view.columns.forEach((column, depth) => {
    if (column.some((listed) => listed.selected)) {
      deepest = depth;
    }
  });
  view.columns.forEach((column, depth) => {
    const shown = el("div", "column");
    if (column.length === 0) {
      shown.append(el("p", "none", "Empty"));
    }
    for (const listed of column) {
      if (listed.heading !== null) {
        shown.append(el("h3", "column-heading", listed.heading));
      }
      shown.append(entry(listed, depth, depth < deepest, on));
    }
    walk.append(shown);
  });
  if (view.preview !== null) {
    walk.append(preview(view.preview, on));
  }
  return walk;
}
