// Draws the current folder's bookmark list.
// Every text reaches the page as text; the handlers the page passes in carry out what a person does.
// The list draws the rows in sight from the page cache.

import { el, find, icon } from "./dom.js";
import type { Line, Rows, Section } from "./rows.js";
import { Frame, Geometry, sections } from "./rows.js";
import type {
	CollectionCount,
	DragMaterial,
	Extend,
	Item,
	ListingView,
	RowView,
	ScopeName,
	Target,
	View,
} from "./types.js";

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
	cancel(): void;
	/** Selects the row at a place, as a click with its modifiers asks. */
	select(id: string, place: number, extend: Extend): void;
	open(id: string): void;
	/** Opens the menu of the shortcut at a place, which acts on the selection when the selection takes it. */
	menu(
		event: MouseEvent | KeyboardEvent,
		item: Item,
		place: number,
		selected: boolean,
	): void;
	collectionMenu(
		event: MouseEvent | KeyboardEvent,
		id: string,
		name: string,
		mark: string,
	): void;
	/** Opens the system folder's menu. */
	trashMenu(event: MouseEvent | KeyboardEvent): void;
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
	copy(link: string): void;
	edit(item: Item): void;
	icon(image: HTMLImageElement, name: string): void;
}

const DATE = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const DATE_WITH_YEAR = new Intl.DateTimeFormat("en", {
	year: "numeric",
	month: "short",
	day: "numeric",
});
const MOMENT = new Intl.DateTimeFormat("en", {
	year: "numeric",
	month: "short",
	day: "numeric",
	hour: "numeric",
	minute: "2-digit",
});
const NUMBER = new Intl.NumberFormat("en");
const HALVES = 2;

/** The height of a row of the list, which the style sheet draws and the geometry counts. */
let rowHeight = 48;
/** The height of a heading of the list. */
let headingHeight = 56;
/** How far past the band in sight the page draws lines, so a short scroll shows rows at once. */
const OVERSCAN = 480;

/** Sets the heights the style sheet draws with from the ones the geometry counts with, so both hold the same numbers. */
export function applyMetrics(): boolean {
	const row = find("row-metrics", HTMLElement).getBoundingClientRect().height;
	const heading = find("heading-metrics", HTMLElement).getBoundingClientRect()
		.height;
	const changed = row !== rowHeight || heading !== headingHeight;
	rowHeight = row;
	headingHeight = heading;
	document.documentElement.style.setProperty("--row-height", `${row}px`);
	document.documentElement.style.setProperty(
		"--list-heading-height",
		`${heading}px`,
	);
	return changed;
}

/** A day's date, with its year when it is not this one. */
export function day(time: number): string {
	const date = new Date(time);
	return date.getFullYear() === new Date().getFullYear()
		? DATE.format(date)
		: DATE_WITH_YEAR.format(date);
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
	return links === 0
		? folders(folderCount)
		: `${folders(folderCount)} and ${shortcuts(links)}`;
}

/** Whether a value read from a drag is a target of a move. */
function isTarget(value: unknown): value is Target {
	if (typeof value !== "object" || value === null || !("kind" in value)) {
		return false;
	}
	return (
		value.kind === "selection" ||
		(value.kind === "drag" &&
			"token" in value &&
			typeof value.token === "string") ||
		(value.kind === "ids" &&
			"ids" in value &&
			Array.isArray(value.ids) &&
			value.ids.every((id: unknown) => typeof id === "string"))
	);
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
function dropTarget(
	entry: HTMLElement,
	label: string,
	take: (target: Target) => void,
): void {
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
function filing(
	entry: HTMLElement,
	label: string,
	collection: string | null,
	on: Handlers,
): void {
	dropTarget(entry, label, (target) => on.dropOnCollection(target, collection));
	if (collection !== null) {
		springLoaded(entry, collection, on);
	}
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
		if (stopped !== null) {
			void on.hover(stopped.token, null).catch(on.failed);
		}
		if (stopSpring === stop) {
			stopSpring = null;
		}
	};
	entry.addEventListener("dragover", (event) => {
		if (!carriesEntries(event) || activeDrag === null || pending !== null) {
			return;
		}
		stopSpring?.();
		const asked = { token: activeDrag.token };
		pending = asked;
		stopSpring = stop;
		void on
			.hover(asked.token, folder)
			.then((delay) => {
				if (pending !== asked || delay === null) {
					return;
				}
				timer = window.setTimeout(() => {
					if (pending === asked && entry.isConnected) {
						on.spring(asked.token, folder);
					}
				}, delay);
			})
			.catch(on.failed);
	});
	entry.addEventListener("dragleave", (event) => {
		if (
			!(event.relatedTarget instanceof Node) ||
			!entry.contains(event.relatedTarget)
		) {
			stop();
		}
	});
	entry.addEventListener("drop", stop);
}

function draggable(
	element: HTMLElement,
	target: () => Target,
	on: Handlers,
): void {
	let ready: DragMaterial | null = null;
	let request: object | null = null;
	element.draggable = true;
	element.addEventListener("pointerdown", (event) => {
		if (event.button !== 0 || !element.draggable) {
			return;
		}
		if (
			event.target instanceof Element &&
			event.target.closest(
				"button:not(.row-open), input, label, select, textarea",
			) !== null
		)
			return;
		const asked = {};
		request = asked;
		ready = null;
		void on
			.prepareDrag(target())
			.then((prepared) => {
				if (request === asked && element.isConnected) {
					ready = prepared;
				} else {
					on.endDrag(prepared.token);
				}
			})
			.catch(on.failed);
	});
	element.addEventListener("dragstart", (event) => {
		if (!element.draggable) {
			return;
		}
		if (ready === null || event.dataTransfer === null) {
			event.preventDefault();
			on.failed("The selection is still preparing to drag. Try again.");
			return;
		}
		activeDrag = ready;
		event.dataTransfer.setData(
			DRAGGED,
			JSON.stringify({ kind: "drag", token: ready.token }),
		);
		if (ready.links !== "") {
			event.dataTransfer.setData("text/uri-list", ready.links);
			event.dataTransfer.setData("text/plain", ready.links);
		}
		event.dataTransfer.effectAllowed = "copyMove";
		if (ready.count > 1) {
			const badge = el(
				"div",
				"drag-badge",
				`${NUMBER.format(ready.count)} items`,
			);
			document.body.append(badge);
			const bounds = badge.getBoundingClientRect();
			event.dataTransfer.setDragImage(
				badge,
				bounds.width / HALVES,
				bounds.height / HALVES,
			);
			window.requestAnimationFrame(() => badge.remove());
		}
	});
	element.addEventListener("dragend", () => {
		stopSpring?.();
		request = null;
		if (ready !== null) {
			on.endDrag(ready.token);
		}
		if (activeDrag?.token === ready?.token) {
			activeDrag = null;
		}
		ready = null;
	});
	element.addEventListener("pointerup", () => {
		if (activeDrag !== null) return;
		request = null;
		if (ready !== null) on.endDrag(ready.token);
		ready = null;
	});
}

/** Whether the window stands in the trash, whose rows neither move nor take a name typed over them. */
export function inTrash(view: View): boolean {
	return view.scope.kind === "trash";
}

/** A folder's glyph, or the mark a person gave it in its place. */
function folderGlyph(mark: string): Element {
	const glyph = mark === "" ? icon("folder") : el("span", "mark", mark);
	glyph.setAttribute("aria-hidden", "true");
	return glyph;
}

/** Lets a folder be dragged onto another folder. */
function draggableFolder(
	element: HTMLElement,
	target: () => Target,
	on: Handlers,
): void {
	draggable(element, target, on);
}

function contextual(
	element: HTMLElement,
	show: (event: MouseEvent | KeyboardEvent) => void,
): void {
	element.addEventListener("contextmenu", show);
	element.addEventListener("keydown", (event) => {
		if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey))
			show(event);
	});
}

/** A site's icon, with a link icon while no valid image is available. */
function favicon(name: string | null, on: Handlers): HTMLElement {
	const holder = el("span", "site-icon");
	holder.setAttribute("aria-hidden", "true");
	const fallback = icon("link");
	holder.append(fallback);
	if (name === null) return holder;
	const image = el("img", "favicon");
	image.alt = "";
	image.hidden = true;
	// A drag that starts on the icon moves the shortcut, as one that starts beside it does, rather than the picture.
	image.draggable = false;
	image.addEventListener("load", () => {
		fallback.setAttribute("hidden", "");
		image.hidden = false;
	});
	image.addEventListener("error", () => {
		image.hidden = true;
		fallback.removeAttribute("hidden");
	});
	holder.append(image);
	on.icon(image, name);
	return holder;
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
	place: number;
	selected: boolean;
	cut: boolean;
	count: number;
}

/** Marks an element of a row with what the window reads back of it: its identifier, its place, and whether the selection takes it. */
function marked(element: HTMLElement, id: string, spot: Spot): void {
	element.setAttribute("data-id", id);
	element.setAttribute("data-place", String(spot.place));
	element.setAttribute("data-selected", String(spot.selected));
	element.classList.toggle("cut", spot.cut);
}

/** Preserves an existing single selection while preparing a double-click. */
function selecting(
	element: HTMLElement,
	id: string,
	spot: Spot,
	on: Handlers,
): void {
	element.addEventListener("click", (event) => {
		element
			.querySelector<HTMLElement>(".row-open")
			?.focus({ preventScroll: true });
		const how = extend(event);
		const alone = spot.count === 1 && spot.selected;
		if (how !== "replace" || !alone) {
			on.select(id, spot.place, how);
		}
	});
}

/** Wires what a person does to one shortcut: select, open, drag outside the trash, and the menu. */
function interactive(
	element: HTMLElement,
	item: Item,
	spot: Spot,
	view: View,
	on: Handlers,
): void {
	marked(element, item.id, spot);
	if (!inTrash(view)) {
		draggable(
			element,
			() =>
				spot.selected ? { kind: "selection" } : { kind: "ids", ids: [item.id] },
			on,
		);
	}
	selecting(element, item.id, spot, on);
	opening(element, view, () => on.open(item.id));
	contextual(element, (event) =>
		on.menu(event, item, spot.place, spot.selected),
	);
}

/** Wires selection, opening, dragging, dropping, and the menu for a folder row. */
function folderInteractive(
	element: HTMLElement,
	folder: CollectionCount,
	spot: Spot,
	view: View,
	on: Handlers,
): void {
	marked(element, folder.id, spot);
	selecting(element, folder.id, spot, on);
	opening(element, view, () => on.open(folder.id));
	contextual(element, (event) =>
		on.collectionMenu(event, folder.id, folder.name, folder.mark),
	);
	// A folder in the trash takes no drop and moves only out of the trash through its menu.
	if (!inTrash(view)) {
		filing(element, "Move here", folder.id, on);
		draggableFolder(
			element,
			() =>
				spot.selected
					? { kind: "selection" }
					: { kind: "ids", ids: [folder.id] },
			on,
		);
	}
}

function empty(view: View): HTMLElement {
	const made = el("div", "empty");
	const heading = inTrash(view) ? "Trash is empty" : "No bookmarks yet";
	const help = inTrash(view)
		? "Bookmarks moved to Trash can be restored here."
		: "Right-click or press Shift+F10 to add a bookmark or folder, or drop a link here.";
	made.append(el("h2", "", heading), el("p", "", help));
	return made;
}

/** The heading of the folders, which sets them apart from the shortcuts after them. */
const FOLDERS = "Folders";
/** The heading of the shortcuts after the folders, when no grouping heads them. */
const SHORTCUTS = "Shortcuts";

/** Divides the listing into folder and shortcut sections. */
function listingSections(listing: ListingView, named: boolean): Section[] {
	const links = listing.total - listing.folders;
	const both = named && listing.folders > 0 && links > 0;
	const marks: Section[] = [];
	if (listing.folders > 0) {
		marks.push({
			start: 0,
			count: listing.folders,
			label: both ? FOLDERS : null,
		});
	}
	if (listing.groups.length > 0) {
		marks.push(
			...listing.groups.map((group) => ({
				start: group.place,
				count: group.count,
				label: group.label,
			})),
		);
	} else {
		marks.push({
			start: listing.folders,
			count: links,
			label: both ? SHORTCUTS : null,
		});
	}
	return sections(listing.total, marks);
}

/** A heading the shortcuts after it fall under, with how many they are. */
function groupHeading(label: string, count: number): HTMLElement {
	const heading = el("h3", "group-heading", label);
	heading.append(el("span", "count", NUMBER.format(count)));
	return heading;
}

/** The places of a listing's rows: from the first to past the last. */
export interface Span {
	readonly first: number;
	readonly end: number;
}

/** A listing drawn in the content: the element that holds it, its geometry, and how a scroll of the content shows its lines. */
export interface Drawn {
	readonly element: HTMLElement;
	readonly key: string;
	readonly geometry: Geometry;
	update(view: View): void;
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

function anchored(
	frame: Frame,
	inset: number,
	line: { readonly top: number; readonly offset: number },
	client: number,
): number {
	const top = line.top - line.offset;
	return inset + (top < 0 ? top : frame.scroll(top, client));
}

/** The band of a frame that a scroll of the content shows, for a frame set an offset below the content's top. */
function seen(
	frame: Frame,
	offset: number,
	scroll: number,
	client: number,
): Seen {
	const own = Math.max(0, scroll - offset);
	return { own, client, top: frame.top(own, client) };
}

/** The scroll of the content that brings a line of a frame into sight, nearest the scroll it has. */
function revealed(
	frame: Frame,
	offset: number,
	line: { readonly top: number; readonly height: number },
	band: Seen,
): number {
	const top =
		line.top < band.top
			? line.top
			: line.top + line.height > band.top + band.client
				? line.top + line.height - band.client
				: band.top;
	return offset + frame.scroll(top, band.client);
}

class Band {
	private drawn = "";

	changed(
		lines: readonly { readonly top: number }[],
		version: number,
	): boolean {
		const signature = `${lines[0]?.top ?? -1}:${lines.at(-1)?.top ?? -1}:${lines.length}:${version}`;
		const changed = signature !== this.drawn;
		this.drawn = signature;
		return changed;
	}
}

function nameCell(...children: Node[]): HTMLDivElement {
	const cell = el("div", "name");
	cell.append(...children);
	return cell;
}

function opening(element: HTMLElement, view: View, activate: () => void): void {
	element.addEventListener("dblclick", (event) => {
		if (event.button === 0 && extend(event) === "replace" && !inTrash(view))
			activate();
	});
}

function openButton(
	name: string,
	id: string,
	spot: Spot,
	activate: () => void,
): HTMLButtonElement {
	const made = el("button", "row-open");
	made.type = "button";
	made.title = name;
	made.setAttribute("aria-label", `Open ${name}`);
	made.setAttribute("data-focus-key", `open:${id}`);
	made.setAttribute("aria-describedby", `selection-${id}`);
	made.append(el("span", "item-name", name));
	const state = el(
		"span",
		"sr-only",
		spot.selected ? "Selected" : "Not selected",
	);
	state.id = `selection-${id}`;
	made.append(state);
	made.addEventListener("click", (event) => {
		if (event.detail === 0) {
			event.stopPropagation();
			activate();
		}
	});
	return made;
}

function row(item: Item, spot: Spot, view: View, on: Handlers): HTMLLIElement {
	const line = el("li", "bookmark-row");
	const title = inTrash(view)
		? trashedLabel(item.title, item.id, spot)
		: openButton(item.title, item.id, spot, () => on.open(item.id));
	title.title = item.link;
	const context = el(
		"span",
		"sr-only",
		item.collection === null ? "Bookmark" : `Bookmark in ${item.collection}`,
	);
	context.id = `context-${item.id}`;
	title.setAttribute("aria-describedby", `${context.id} selection-${item.id}`);
	title.append(context);
	title.prepend(favicon(item.icon, on));
	line.append(nameCell(title));
	interactive(line, item, spot, view, on);
	return line;
}

function folderRow(
	folder: CollectionCount,
	spot: Spot,
	view: View,
	on: Handlers,
): HTMLLIElement {
	const line = el("li", "folder-row");
	const title = inTrash(view)
		? trashedLabel(folder.name, folder.id, spot)
		: openButton(folder.name, folder.id, spot, () => on.open(folder.id));
	title.prepend(folderGlyph(folder.mark));
	const context = el("span", "sr-only", "Folder");
	context.id = `context-${folder.id}`;
	title.append(context);
	title.setAttribute(
		"aria-describedby",
		`${context.id} selection-${folder.id}`,
	);
	line.append(nameCell(title));
	folderInteractive(line, folder, spot, view, on);
	return line;
}

function trashedLabel(name: string, id: string, spot: Spot): HTMLSpanElement {
	const label = el("span", "row-open");
	label.tabIndex = 0;
	label.setAttribute("data-focus-key", `open:${id}`);
	label.append(el("span", "item-name", name));
	const state = el(
		"span",
		"sr-only",
		spot.selected ? "Selected" : "Not selected",
	);
	state.id = `selection-${id}`;
	label.append(state);
	return label;
}

function trashRow(on: Handlers): HTMLLIElement {
	const line = el("li", "system-folder");
	const open = el("button", "row-open");
	open.type = "button";
	open.setAttribute("aria-label", "Open Trash");
	open.setAttribute("data-focus-key", "open:trash");
	open.setAttribute("aria-describedby", "selection-trash");
	const state = el("span", "sr-only", "Not selected");
	state.id = "selection-trash";
	open.append(icon("trash"), el("span", "item-name", "Trash"), state);
	const activate = (): void => on.scope({ kind: "trash" });
	open.addEventListener("click", (event) => {
		if (event.detail === 0) {
			event.stopPropagation();
			activate();
		}
	});
	line.addEventListener("click", () => {
		open.focus({ preventScroll: true });
		on.cancel();
		line.setAttribute("data-selected", "true");
		state.textContent = "Selected";
	});
	open.addEventListener("keydown", (event) => {
		if (event.key === "Escape") {
			line.removeAttribute("data-selected");
			state.textContent = "Not selected";
		}
	});
	line.addEventListener("dblclick", (event) => {
		if (event.button === 0 && extend(event) === "replace") activate();
	});
	contextual(line, on.trashMenu);
	dropTarget(line, "Move to Trash", (target) => on.trash(target));
	line.append(nameCell(open));
	return line;
}

function spacer(): HTMLLIElement {
	const line = el("li", "spacer");
	line.setAttribute("role", "presentation");
	line.setAttribute("aria-hidden", "true");
	return line;
}

function lineHeight(line: Line): number {
	return line.kind === "heading" ? headingHeight : rowHeight;
}

interface CachedRow {
	readonly signature: string;
	readonly element: HTMLLIElement;
	readonly spot: Spot;
}

/** One folder's paged contents, with stable controls while selection changes. */
export function list(
	view: View,
	held: Rows<RowView>,
	on: Handlers,
): Drawn | HTMLElement {
	const atRoot = view.scope.kind === "all";
	if (view.listing.total === 0 && !atRoot) return empty(view);
	const geometry = new Geometry(listingSections(view.listing, false), {
		across: 1,
		pitch: rowHeight,
		heading: headingHeight,
	});
	const listing = el("div", "listing");
	const body = el("ul", "list");
	body.setAttribute("role", "list");
	body.setAttribute("aria-label", "Folder contents");
	body.setAttribute("aria-describedby", "list-help");
	const instructions = el(
		"p",
		"sr-only",
		"Click a row to select it. Double-click to open a bookmark or folder, or focus its name and press Enter or Space. Press Shift+F10 or the Menu key for item actions. Use Control-click or Command-click to toggle selection, or Shift-click to select a range.",
	);
	instructions.id = "list-help";
	listing.append(instructions, body);
	if (atRoot && view.listing.total === 0) listing.append(empty(view));
	const system = atRoot ? trashRow(on) : null;
	const extra = system === null ? 0 : 1;
	if (system !== null) {
		system.setAttribute("aria-setsize", String(view.listing.total + extra));
		system.setAttribute("aria-posinset", "1");
	}
	const frame = new Frame(geometry.height);
	const before = spacer();
	const after = spacer();
	const band = new Band();
	const cache = new Map<string, CachedRow>();
	const offset = (): number =>
		body.offsetTop + (system === null ? 0 : rowHeight);
	const makeRow = (
		shown: RowView | undefined,
		place: number,
	): HTMLLIElement => {
		if (shown === undefined || shown.kind === "gone") {
			const line = el("li", "placeholder");
			line.setAttribute("aria-hidden", "true");
			return line;
		}
		const id = shown.kind === "folder" ? shown.folder.id : shown.item.id;
		const signature = JSON.stringify({ ...shown, selected: false, cut: false });
		const previous = cache.get(id);
		const spot = previous?.spot ?? {
			place,
			selected: shown.selected,
			cut: shown.cut,
			count: view.selection.count,
		};
		Object.assign(spot, {
			place,
			selected: shown.selected,
			cut: shown.cut,
			count: view.selection.count,
		});
		const line =
			previous?.signature === signature
				? previous.element
				: shown.kind === "folder"
					? folderRow(shown.folder, spot, view, on)
					: row(shown.item, spot, view, on);
		marked(line, id, spot);
		const state = line.querySelector(`#selection-${CSS.escape(id)}`);
		if (state !== null)
			state.textContent = spot.selected ? "Selected" : "Not selected";
		line.setAttribute("aria-setsize", String(view.listing.total + extra));
		line.setAttribute("aria-posinset", String(place + extra + 1));
		cache.set(id, { signature, element: line, spot });
		return line;
	};
	return {
		element: listing,
		key: view.listing.key,
		geometry,
		update(changed) {
			view = changed;
			if (view.selection.count > 0 && system !== null) {
				system.removeAttribute("data-selected");
				const state = system.querySelector("#selection-trash");
				if (state !== null) state.textContent = "Not selected";
			}
		},
		show(scroll, client) {
			const sight = seen(frame, offset(), scroll, client);
			const lines = geometry.lines(
				sight.top - OVERSCAN,
				sight.top + client + OVERSCAN,
			);
			if (band.changed(lines, held.version)) {
				const focused = body.contains(document.activeElement)
					? document.activeElement?.getAttribute("data-focus-key")
					: null;
				const shown = lines.map((line) => {
					if (line.kind === "rows")
						return makeRow(held.get(line.first), line.first);
					const heading = el("li", "group-row");
					heading.setAttribute("role", "presentation");
					heading.append(
						groupHeading(line.section.label ?? "", line.section.count),
					);
					return heading;
				});
				body.replaceChildren(
					...(system === null ? [] : [system]),
					before,
					...shown,
					after,
				);
				for (const [id, cached] of cache)
					if (!cached.element.isConnected) cache.delete(id);
				if (focused !== null && focused !== undefined)
					(
						body.querySelector<HTMLElement>(
							`[data-focus-key="${CSS.escape(focused)}"]`,
						) ?? find("content", HTMLElement)
					).focus({ preventScroll: true });
			}
			const first = lines[0];
			const above =
				first === undefined
					? 0
					: Math.max(0, frame.place(first.top, sight.own, client));
			const drawn = lines.reduce((sum, line) => sum + lineHeight(line), 0);
			before.style.setProperty("--height", `${above}px`);
			after.style.setProperty(
				"--height",
				`${Math.max(0, frame.height - above - drawn)}px`,
			);
			if (view.listing.total === 0 && listing.querySelector(".empty") === null)
				listing.append(empty(view));
			return geometry.span(sight.top, sight.top + client);
		},
		span(scroll, client) {
			const sight = seen(frame, offset(), scroll, client);
			return geometry.span(sight.top, sight.top + client);
		},
		reveal(placed, scroll, client) {
			const line = { top: geometry.top(placed), height: rowHeight };
			return revealed(
				frame,
				offset(),
				line,
				seen(frame, offset(), scroll, client),
			);
		},
		anchor(placed, pin, client) {
			return anchored(
				frame,
				offset(),
				{ top: geometry.top(placed), offset: pin },
				client,
			);
		},
	};
}
