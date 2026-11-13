// The window's side of the library: it sends what a person does to the library and draws the view it gets back.
// It decides nothing the library decides; every shortcut's text reaches the page as text, never as markup.
// The page holds the rows around the place in sight and asks the library for the others as a person scrolls, so a listing of millions draws as fast as one of a few.

import { button, el, find, icon } from "./dom.js";
import type { Drawn, Handlers } from "./draw.js";
import { applyMetrics, DRAGGED, described, inTrash, list } from "./draw.js";
import { Rows } from "./rows.js";
import type { FormResult, MenuChoice } from "./sheets.js";
import {
	AddSheet,
	AskSheet,
	EditSheet,
	MarkSheet,
	Menu,
	MoveSheet,
} from "./sheets.js";
import type {
	Choice,
	DragMaterial,
	EmojiGroup,
	Intent,
	Item,
	Key,
	Progress,
	RowsView,
	RowView,
	Target,
	View,
} from "./types.js";
import { invoke, listen, listenProgress, listenText } from "./types.js";

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
const NOTICE_MS = 5_000;

const content = find("content", HTMLElement);
const notice = find("notice", HTMLElement);
const noticeText = find("notice-text", HTMLElement);
const resultsStatus = find("results-status", HTMLElement);
const dropHint = find("drop-hint", HTMLElement);
const trailNav = find("trail", HTMLElement);
/** The name of the root in the collection trail. */
const ROOT_NAME = "Bookmarks";
/** The parent folder's offset from the end of the collection trail. */
const PARENT_STEP = -2;
let current: View | null = null;
let tracedClickTarget: EventTarget | null = null;
let tracedClickRow: HTMLElement | null = null;
for (const kind of ["pointerdown", "pointerup", "click", "dblclick"] as const) {
	document.addEventListener(
		kind,
		(event: MouseEvent): void => {
			const target = event.target instanceof Element ? event.target : null;
			const row =
				target?.closest<HTMLElement>("li[data-id], li.system-folder") ?? null;
			console.info(
				"POINTER",
				JSON.stringify({
					kind: event.type,
					time: performance.now(),
					button: event.button,
					detail: event.detail,
					shift: event.shiftKey,
					control: event.ctrlKey,
					meta: event.metaKey,
					x: event.clientX,
					y: event.clientY,
					target: target?.tagName,
					row: row?.className,
					place: row?.getAttribute("data-place"),
					top: row?.getBoundingClientRect().top,
					bottom: row?.getBoundingClientRect().bottom,
					sameTarget: event.target === tracedClickTarget,
					sameRow: row === tracedClickRow,
					previousTargetConnected:
						tracedClickTarget instanceof Node && tracedClickTarget.isConnected,
					previousRowConnected: tracedClickRow?.isConnected,
					key: current?.listing.key,
					scope: current?.scope.kind,
				}),
			);
			if (event.type === "click") {
				tracedClickTarget = event.target;
				tracedClickRow = row;
			}
		},
		true,
	);
}
/** The calls in flight, in order, so the library reads what a person did in the order they did it. */
let queue: Promise<void> = Promise.resolve();
/** The icons drawn so far by name, and the names asked for. */
const drawnIcons = new Map<string, string>();
const askedIcons = new Set<string>();
/** The images waiting for each icon. */
const waiting = new Map<string, HTMLImageElement[]>();
/** The cached rows around the visible list. */
const listingRows = new Rows<RowView>();
let drawnListing: Drawn | null = null;
/** How many views the page drew, so a page that answers an older one changes nothing. */
let drawnViews = 0;
/** Whether a call for rows is in flight, whether another band waits for one, and the last call, which goes once for a view. */
let fetching = false;
let wanting = false;
let lastCall = "";
/** Whether a view that a changed store asks for waits in the queue already. */
let refreshing = false;
let restTimer: number | undefined;
type NoticeKind = "completion" | "error" | "progress";
let noticeKind: NoticeKind = "completion";
let noticeTimer: number | undefined;

function pauseNotice(): void {
	window.clearTimeout(noticeTimer);
}

function hideNotice(): void {
	pauseNotice();
	notice.hidden = true;
	noticeText.textContent = "";
}

function expireNotice(): void {
	pauseNotice();
	if (
		notice.hidden ||
		noticeKind !== "completion" ||
		document.hidden ||
		notice.matches(":hover, :focus-within")
	)
		return;
	noticeTimer = window.setTimeout(hideNotice, NOTICE_MS);
}

function showNotice(view: View, feedback: "command" | "form"): void {
	if (view.notice === null || (feedback === "form" && view.notice.error)) {
		hideNotice();
		return;
	}
	announce(view.notice.text, view.notice.error ? "error" : "completion");
}

function announce(text: string, kind: NoticeKind = "completion"): void {
	noticeKind = kind;
	const error = kind === "error";
	noticeText.textContent = error ? `Error: ${text}` : text;
	notice.classList.toggle("error", error);
	notice.hidden = false;
	expireNotice();
}

/** Shows how far a move or a removal came, until the view after it arrives. */
function showProgress(progress: Progress): void {
	announce(progress.text, "progress");
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

/** Draws the collection trail with a navigation action for each parent. */
function drawFolderTrail(view: View): void {
	const steps = inTrash(view) ? [{ id: null, name: "Trash" }] : view.trail;
	trailNav.hidden = steps.length === 0;
	if (steps.length === 0) {
		trailNav.replaceChildren();
		return;
	}
	const top = button(ROOT_NAME, "");
	top.addEventListener("click", () =>
		send({ kind: "scope", scope: { kind: "all" } }),
	);
	const path = el("ol");
	const root = el("li");
	root.append(top);
	path.append(root);
	for (const [place, step] of steps.entries()) {
		const item = el("li");
		if (place === steps.length - 1) {
			const here = el("span", "here", step.name);
			here.setAttribute("aria-current", "page");
			item.append(here);
		} else if (step.id !== null) {
			const id = step.id;
			const made = button(step.name, "");
			made.addEventListener("click", () =>
				send({
					kind: "scope",
					scope: { kind: "collection", id },
				}),
			);
			item.append(made);
		}
		path.append(item);
	}
	trailNav.replaceChildren(path);
}

/** Navigates to the parent collection. */
function up(): void {
	if (current === null || (current.trail.length === 0 && !inTrash(current))) {
		return;
	}
	const parent = current.trail.at(PARENT_STEP);
	send({
		kind: "scope",
		scope:
			parent === undefined
				? { kind: "all" }
				: { kind: "collection", id: parent.id },
	});
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
		failed(error);
	} finally {
		fetching = false;
	}
	if (wanting) {
		wanting = false;
	}
	showListing();
}

/** Asks the library for the entries of a column around a span in sight, and draws them once they arrive. */

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
	readonly anchor: {
		readonly id: string;
		readonly place: number;
		readonly offset: number;
	} | null;
}

function pinViewport(
	scroller: HTMLElement,
	identity: string,
	column: number | null,
): ViewportPin {
	const area = scroller.getBoundingClientRect();
	const top = area.top + scroller.clientTop;
	const covered = Math.max(
		top,
		scroller.querySelector("thead")?.getBoundingClientRect().bottom ?? top,
	);
	for (const entry of scroller.querySelectorAll<HTMLElement>(
		"[data-id][data-place]",
	)) {
		const id = entry.getAttribute("data-id") ?? undefined;
		const place = Number(entry.getAttribute("data-place") ?? undefined);
		const bounds = entry.getBoundingClientRect();
		if (
			id !== undefined &&
			Number.isSafeInteger(place) &&
			place >= 0 &&
			bounds.bottom > covered &&
			bounds.top < top + scroller.clientHeight &&
			bounds.right > area.left &&
			bounds.left < area.right
		) {
			return {
				identity,
				column,
				scroll: scroller.scrollTop,
				anchor: { id, place, offset: bounds.top - top },
			};
		}
	}
	return { identity, column, scroll: scroller.scrollTop, anchor: null };
}

function viewportPins(): readonly ViewportPin[] {
	return current === null
		? []
		: [pinViewport(content, current.listing.identity, null)];
}

/** Draws the bookmark list from the cached rows. */
function drawContent(
	view: View,
	answers: boolean,
	pins: readonly ViewportPin[] = [],
): void {
	if (drawnListing !== null && drawnListing.key === view.listing.key) {
		drawnListing.update(view);
		showListing();
		if (answers && view.listing.reveal !== null) {
			content.scrollTop = drawnListing.reveal(
				view.listing.reveal,
				content.scrollTop,
				content.clientHeight,
			);
		}
		return;
	}
	drawnListing = null;
	const drawn = list(view, listingRows, handlers);
	if (drawn instanceof HTMLElement) {
		content.replaceChildren(drawn);
		return;
	}
	content.replaceChildren(drawn.element);
	drawnListing = drawn;
	if (view.listing.fresh) {
		content.scrollTop = 0;
	}
	const pin = pins.find(
		(pin) => pin.column === null && pin.identity === view.listing.identity,
	);
	if (!answers && pin !== undefined) {
		content.scrollTop = pin.scroll;
		if (pin.anchor !== null) {
			const place =
				view.listing.anchor?.id === pin.anchor.id
					? view.listing.anchor.place
					: pin.anchor.place;
			content.scrollTop = drawn.anchor(
				place,
				pin.anchor.offset,
				content.clientHeight,
			);
		}
	}
	const reveal = view.listing.reveal;
	if (answers && reveal !== null) {
		content.scrollTop = drawn.reveal(
			reveal,
			content.scrollTop,
			content.clientHeight,
		);
	}
	showListing();
}

/** Draws a view; one that answers no intent of the person's keeps the scroll and the notice on screen, unless the notice is new. */
function draw(
	view: View,
	answers = true,
	pins: readonly ViewportPin[] = [],
	feedback: "command" | "form" = "command",
): void {
	console.info(
		"DRAW",
		JSON.stringify({
			key: view.listing.key,
			previousKey: current?.listing.key,
			scope: view.scope.kind,
			selection: view.selection.count,
			previousTargetConnected:
				tracedClickTarget instanceof Node && tracedClickTarget.isConnected,
			previousRowConnected: tracedClickRow?.isConnected,
		}),
	);
	const shown = current?.notice?.text;
	const previousScope = current?.scope;
	const locationChanged =
		previousScope !== undefined &&
		JSON.stringify(previousScope) !== JSON.stringify(view.scope);
	const focusedControl =
		document.activeElement?.getAttribute("data-focus-key") ?? null;
	const focusedInResults = content.contains(document.activeElement);
	current = view;
	drawnViews += 1;
	refreshing = false;
	listingRows.reset(
		view.listing.key,
		view.listing.total,
		view.listing.first,
		view.listing.rows,
	);
	waiting.clear();
	drawFolderTrail(view);
	drawContent(view, answers, pins);
	if (JSON.stringify(previousScope) !== JSON.stringify(view.scope)) {
		resultsStatus.textContent =
			view.listing.total === 0
				? "No bookmarks in this folder."
				: `${view.listing.total} ${view.listing.total === 1 ? "item" : "items"} shown.`;
	}
	if (focusedControl !== null) {
		const replacement = document.querySelector<HTMLElement>(
			`[data-focus-key="${CSS.escape(focusedControl)}"]`,
		);
		const successor =
			view.focus === null
				? null
				: content.querySelector<HTMLElement>(
						`[data-focus-key="open:${CSS.escape(view.focus)}"]`,
					);
		(replacement ?? (focusedInResults ? (successor ?? content) : null))?.focus({
			preventScroll: true,
		});
	}
	if (locationChanged || previousScope === undefined)
		content.focus({ preventScroll: true });
	if (answers || view.notice?.text !== shown) {
		showNotice(view, feedback);
	}
	historyMenu();
	void fetchIcons();
}

/** Shows a failure of a call to the library. */
function failed(error: unknown): void {
	announce(String(error), "error");
}

/** Names the next history steps while fields keep their own text editing. */
function historyMenu(): void {
	void invoke("history_menu", {
		state: {
			undo: current?.undo ?? null,
			redo: current?.redo ?? null,
			field: typing(document.activeElement),
			enabled: fileCommands(),
			inTrash: trashShown(),
		},
	}).catch(failed);
}

document.addEventListener("focusin", historyMenu);
document.addEventListener("focusout", () => queueMicrotask(historyMenu));

function fileCommands(): string[] {
	if (
		typing(document.activeElement) ||
		document.querySelector("dialog[open]") !== null
	)
		return [];
	const commands: string[] = [];
	const row = focusedRow();
	if (!trashShown()) {
		commands.push("new-link", "new-collection");
		if (row?.kind === "shortcut")
			commands.push("open", "edit-bookmark", "copy-url");
		if (row?.kind === "folder")
			commands.push("open", "rename-folder", "move-folder", "mark-folder");
	}
	if (row !== null || (current?.selection.count ?? 0) > 0) {
		commands.push("trash-selection");
		if (trashShown()) commands.push("put-back");
	}
	if ((current?.trash ?? 0) > 0) commands.push("empty-trash");
	return commands;
}

async function act(intent: Intent): Promise<void> {
	try {
		draw(await invoke<View>("act", { intent }));
	} catch (error: unknown) {
		failed(error);
	}
}

/** Sends an intent after every earlier one, and draws the view it answers with. */
function send(intent: Intent): void {
	queue = queue.then(() => act(intent));
}

function libraryCommand(command: "copy_selection" | "paste"): void {
	queue = queue.then(async () => {
		try {
			draw(await invoke<View>(command));
		} catch (error: unknown) {
			failed(error);
		}
	});
}

function saveIntent(intent: Intent): Promise<FormResult> {
	const saving = queue.then(async (): Promise<FormResult> => {
		const view = await invoke<View>("act", { intent });
		draw(view, true, [], "form");
		return view.notice?.error === true
			? { ok: false, message: view.notice.text }
			: { ok: true };
	});
	queue = saving.then(
		() => undefined,
		() => undefined,
	);
	return saving;
}

/** Draws the view as it stands after every earlier intent, after background icon collection. */
function refresh(): void {
	if (refreshing) {
		return;
	}
	refreshing = true;
	queue = queue.then(async () => {
		try {
			const pins = viewportPins();
			const anchors = pins.flatMap((pin) =>
				pin.anchor === null
					? []
					: [{ identity: pin.identity, column: pin.column, id: pin.anchor.id }],
			);
			draw(await invoke<View>("view_anchored", { anchors }), false, pins);
		} catch (error: unknown) {
			refreshing = false;
			failed(error);
		}
	});
}

function focusedRow(): RowView | null {
	const element =
		document.activeElement instanceof Element
			? document.activeElement.closest<HTMLElement>(
					"li[data-place], li.system-folder",
				)
			: null;
	if (element?.classList.contains("system-folder") === true) return null;
	const place =
		element === null
			? current?.listing.focus
			: Number(element.getAttribute("data-place"));
	if (
		place === null ||
		place === undefined ||
		!Number.isInteger(place) ||
		place < 0
	)
		return null;
	const row = listingRows.get(place);
	if (row === undefined || row.kind === "gone") return null;
	const id = row.kind === "shortcut" ? row.item.id : row.folder.id;
	return id === (element?.getAttribute("data-id") ?? current?.focus)
		? row
		: null;
}

/** The focused bookmark from the current page. */
function focused(): Item | null {
	const row = focusedRow();
	return row?.kind === "shortcut" ? row.item : null;
}

/** How many entries a row of the list holds. */
function across(): number {
	return 1;
}

async function copy(link: string): Promise<void> {
	await invoke("copy", { text: link });
	announce("Copied the URL.");
}

const ask = new AskSheet();
const marking = new MarkSheet((query) =>
	invoke<EmojiGroup[]>("emoji", { query }),
);
const menu = new Menu();
/** The collections a picker finds for what a person typed. */
const pick = (query: string, first: number): Promise<Choice[]> =>
	invoke<Choice[]>("choices", { query, first, limit: PICKED });

function trashShown(): boolean {
	return current !== null && inTrash(current);
}

async function deletionConfirmed(
	total: number,
	folderCount: number,
): Promise<boolean> {
	const answer = await ask.ask({
		title:
			total === 1 && folderCount === 0
				? "Delete this shortcut immediately?"
				: `Delete ${described(total, folderCount)} immediately?`,
		text:
			folderCount === 0
				? "The item is permanently deleted from this library and cannot be restored with Undo."
				: "The items and their contents are permanently deleted from this library and cannot be restored with Undo.",
		action: "Delete",
		answer: null,
	});
	return answer !== null;
}

function confirmDeletion(
	target: Target,
	total: number,
	folderCount: number,
): void {
	queue = queue
		.then(async () => {
			const count =
				target.kind === "selection" ? (current?.selection.count ?? 0) : total;
			const folders =
				target.kind === "selection"
					? (current?.selection.folders ?? 0)
					: folderCount;
			if (count > 0 && (await deletionConfirmed(count, folders)))
				await act({ kind: "remove", target });
		})
		.catch(failed);
}

/** Asks once before emptying the trash, and empties it once the person agrees. */
async function confirmEmptying(): Promise<void> {
	const held = current?.trash ?? 0;
	const answer = await ask.ask({
		title: "Empty the Trash?",
		text: `${held === 1 ? "The item" : `The ${held} items`} in the Trash will be permanently deleted from this library and cannot be restored with Undo.`,
		action: "Empty Trash",
		answer: null,
	});
	if (answer !== null) {
		send({ kind: "emptyTrash" });
	}
}

/** Moves what the selection takes to the trash, or, in the trash, asks before deleting it for good. */
function deleteSelection(immediately: boolean): void {
	queue = queue
		.then(async () => {
			if (current === null || current.selection.count === 0) {
				return;
			}
			const removing = immediately || trashShown();
			if (
				removing &&
				!(await deletionConfirmed(
					current.selection.count,
					current.selection.folders,
				))
			)
				return;
			await act({
				kind: removing ? "remove" : "trash",
				target: { kind: "selection" },
			});
		})
		.catch(failed);
}

const adding = new AddSheet({
	clipboard: () => invoke<string | null>("clipboard"),
	normalize: (link) => invoke<string | null>("normalize_link", { link }),
	title: (link) => invoke<string | null>("title", { link }),
	add: async (link, name, collection) => {
		try {
			await queue;
			const view = await invoke<View>("add", {
				link,
				title: name,
				collection,
			});
			draw(view, true, [], "form");
			return view.notice?.error === true
				? { ok: false, message: view.notice.text }
				: { ok: true };
		} catch (error: unknown) {
			return { ok: false, message: String(error) };
		}
	},
});

const editing = new EditSheet(
	(fields) => saveIntent({ kind: "edit", fields }),
	(id) => send({ kind: "trash", target: { kind: "ids", ids: [id] } }),
	pick,
);

const moving = new MoveSheet(pick, (id, collection) =>
	saveIntent({ kind: "file", target: { kind: "ids", ids: [id] }, collection }),
);

/** The choices of the menu of what waits in the trash: put it back, or delete it for good once the person agrees. */
function trashChoices(
	target: Target,
	total: number,
	folderCount: number,
): MenuChoice[] {
	return [
		{
			label: "Put Back",
			icon: "restore",
			act: () => send({ kind: "putBack", target }),
		},
		"line",
		{
			label: "Delete Immediately…",
			icon: "trash",
			danger: true,
			act: () => void confirmDeletion(target, total, folderCount),
		},
	];
}

/** Opens the add sheet over the collection the window stands in. */
function openAdd(dropped: string | null): void {
	if (
		current !== null &&
		!inTrash(current) &&
		document.querySelector("dialog[open]") === null
	) {
		void adding.open(current.here, dropped);
	}
}

function edit(item: Item): void {
	editing.open(item);
}

/** Asks for a name and makes a collection inside `parent`, or at the top for null. */
async function newFolder(parent: Choice | null): Promise<void> {
	if (trashShown() || document.querySelector("dialog[open]") !== null) return;
	await ask.ask({
		title: parent === null ? "New folder" : `New folder in ${parent.path}`,
		text: "",
		action: "Create",
		answer: "",
		save: (name) =>
			saveIntent({ kind: "newCollection", name, parent: parent?.id ?? null }),
	});
}

function renameFolder(id: string, name: string): void {
	void ask.ask({
		title: "Rename folder",
		text: "",
		action: "Rename",
		answer: name,
		save: (renamed) =>
			saveIntent({ kind: "renameCollection", id, name: renamed }),
	});
}

function markFolder(id: string, name: string, mark: string): void {
	void marking.open(name, mark).then((chosen) => {
		if (chosen !== null) send({ kind: "markCollection", id, mark: chosen });
	});
}

const handlers: Handlers = {
	prepareDrag(target: Target): Promise<DragMaterial> {
		const prepared = queue.then(() =>
			invoke<DragMaterial>("prepare_drag", { target }),
		);
		queue = prepared.then(() => undefined).catch(failed);
		return prepared;
	},
	endDrag: (token) => send({ kind: "endDrag", token }),
	hover: (token, folder) => {
		const hovered = queue.then(() =>
			invoke<number | null>("hover", { token, folder }),
		);
		queue = hovered.then(() => undefined).catch(failed);
		return hovered;
	},
	spring: (token, folder) => send({ kind: "spring", token, folder }),
	failed,
	scope: (scope) => {
		send({ kind: "scope", scope });
	},
	cancel: () => send({ kind: "cancel" }),
	select: (id, place, how) => send({ kind: "select", id, extend: how, place }),
	open: (id) => {
		console.info("OPEN");
		send({ kind: "open", id });
	},
	menu: (event, item, _place, selected) => {
		const selection = current?.selection ?? { count: 1, folders: 0 };
		const [total, folderCount]: readonly [number, number] = selected
			? [selection.count, selection.folders]
			: [1, 0];
		const target: Target = selected
			? { kind: "selection" }
			: { kind: "ids", ids: [item.id] };
		if (trashShown()) {
			menu.show(event, trashChoices(target, total, folderCount));
			return;
		}
		const choices: MenuChoice[] = [
			{
				label: "Open",
				icon: "open",
				act: () => send({ kind: "open", id: item.id }),
			},
			{ label: "Copy URL", icon: "copy", act: () => void copy(item.link) },
			{
				label: "Show in Folder",
				icon: "folder",
				act: () => send({ kind: "showInFolder", id: item.id }),
			},
			"line",
			{ label: "Edit…", icon: "edit", act: () => edit(item) },
			"line",
			{
				label: "Move to Trash",
				icon: "trash",
				act: () => send({ kind: "trash", target }),
			},
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
			{ label: "Open", icon: "folder", act: () => send({ kind: "open", id }) },
			{
				label: "Choose emoji…",
				icon: "emoji",
				act: () => markFolder(id, name, mark),
			},
			{
				label: "Rename…",
				icon: "rename",
				act: () => renameFolder(id, name),
			},
			{
				label: "Move to Folder…",
				icon: "move",
				act: () => {
					moving.open(id, name);
				},
			},
			"line",
			{
				label: "Move to Trash",
				icon: "trash",
				act: () => send({ kind: "trash", target }),
			},
		]);
	},
	trashMenu: (event) => {
		menu.show(event, [
			{
				label: "Open",
				icon: "trash",
				act: () => send({ kind: "scope", scope: { kind: "trash" } }),
			},
			...(current !== null && current.trash > 0
				? [
						"line" as const,
						{
							label: "Empty Trash…",
							icon: "trash",
							danger: true,
							act: () => void confirmEmptying(),
						},
					]
				: []),
		]);
	},
	dropOnCollection: (target, collection) =>
		send({ kind: "file", target, collection }),
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
	copy: (link) => void copy(link),
	edit,
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
	return (
		(target instanceof HTMLInputElement && target.type !== "checkbox") ||
		target instanceof HTMLTextAreaElement ||
		target instanceof HTMLSelectElement
	);
}

/** The commands typed with the command key, or the control key off macOS. */
function command(event: KeyboardEvent): boolean {
	const key = event.key.toLowerCase();
	if (key === "arrowup" && !typing(event.target)) {
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
	} else if (key === "n" && event.shiftKey) {
		void newFolder(current?.here ?? null);
	} else if (key === "n") {
		openAdd(null);
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
	} else if (
		(key === "backspace" || key === "delete") &&
		!typing(event.target)
	) {
		// The command key with delete moves the selection to the trash, and with the option key too deletes it for good, as an explorer's keys do.
		deleteSelection(event.altKey);
	} else {
		return false;
	}
	return true;
}

const TEXT_SCALES: readonly number[] = [
	0.75, 0.875, 1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4,
];
const DEFAULT_SCALE = 2;
let textScale = DEFAULT_SCALE;

function zoom(direction: "in" | "out" | "reset"): void {
	textScale =
		direction === "reset"
			? DEFAULT_SCALE
			: Math.max(
					0,
					Math.min(
						TEXT_SCALES.length - 1,
						textScale + (direction === "in" ? 1 : -1),
					),
				);
	const scale = TEXT_SCALES[textScale] ?? 1;
	document.documentElement.style.fontSize = `${scale * 100}%`;
	window.dispatchEvent(new Event("resize"));
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
		send({ kind: "open", id: null });
		return true;
	}
	if (event.key === "Backspace" || event.key === "Delete") {
		deleteSelection(false);
		return true;
	}
	return false;
}

document.addEventListener("keydown", (event) => {
	if (
		(event.metaKey || event.ctrlKey) &&
		!event.isComposing &&
		["+", "=", "-", "0"].includes(event.key)
	) {
		event.preventDefault();
		zoom(event.key === "0" ? "reset" : event.key === "-" ? "out" : "in");
		return;
	}
	if (
		event.defaultPrevented ||
		event.isComposing ||
		document.querySelector("dialog[open]") !== null
	) {
		return;
	}
	const commanding = event.metaKey || event.ctrlKey;
	if (
		!commanding &&
		(event.key === "Enter" || event.key === " ") &&
		event.target instanceof HTMLElement &&
		(event.target.closest("button") !== null ||
			event.target instanceof HTMLInputElement)
	) {
		return;
	}
	if (commanding && command(event)) {
		event.preventDefault();
		return;
	}
	if (
		event.target === content &&
		(event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey))
	) {
		folderMenu(event);
		return;
	}
	if (typing(event.target) || menu.open) {
		return;
	}
	if (selectionKey(event)) {
		event.preventDefault();
	} else if (
		Array.from(event.key).length === 1 &&
		!commanding &&
		!event.altKey
	) {
		event.preventDefault();
		send({ kind: "type", key: event.key });
	}
});

const ON_MAC = navigator.userAgent.includes("Mac");
document.body.classList.toggle("macos", ON_MAC);
resize();
const dismissNotice = find("notice-dismiss", HTMLButtonElement);
dismissNotice.append(icon("close"));
dismissNotice.addEventListener("click", () => {
	hideNotice();
	content.focus();
});
notice.addEventListener("pointerenter", pauseNotice);
notice.addEventListener("pointerleave", expireNotice);
notice.addEventListener("focusin", pauseNotice);
notice.addEventListener("focusout", () => queueMicrotask(expireNotice));
document.addEventListener("visibilitychange", expireNotice);

/** The link a drop from a browser carries, from its list of links or its text. */
function droppedLink(transfer: DataTransfer): string | null {
	const listed = transfer
		.getData("text/uri-list")
		.split(/\r?\n/u)
		.find((line) => line !== "" && !line.startsWith("#"));
	const link = (listed ?? transfer.getData("text/plain")).trim();
	return link === "" ? null : link;
}

/** Whether a drag brings a file from outside the window, which the window refuses to open. */
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

document.addEventListener("dragover", (event) => {
	if (bringsFile(event)) {
		event.preventDefault();
	}
	if (bringsLink(event)) {
		event.preventDefault();
		dropHint.textContent = DROP_LINK;
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
	if (bringsFile(event)) {
		event.preventDefault();
	}
	if (bringsLink(event) && event.dataTransfer !== null) {
		event.preventDefault();
		openAdd(droppedLink(event.dataTransfer));
	}
});

// The webview's own menu offers to reload or inspect the page, which an app has no use for; a field keeps its editing menu.
document.addEventListener("contextmenu", (event) => {
	if (event.defaultPrevented || typing(event.target)) {
		return;
	}
	event.preventDefault();
	if (event.target instanceof Node && content.contains(event.target)) {
		folderMenu(event);
	}
});

function folderMenu(event: MouseEvent | KeyboardEvent): void {
	content.focus({ preventScroll: true });
	if (trashShown()) {
		if ((current?.trash ?? 0) > 0)
			menu.show(event, [
				{
					label: "Empty Trash…",
					icon: "trash",
					danger: true,
					act: () => void confirmEmptying(),
				},
			]);
		else event.preventDefault();
		return;
	}
	menu.show(event, [
		{ label: "New Bookmark…", icon: "link", act: () => openAdd(null) },
		{
			label: "New Folder…",
			icon: "folder-plus",
			act: () => void newFolder(current?.here ?? null),
		},
	]);
}

/** Carries out a command a person chose from the app's menu. */
function menuCommand(id: string): void {
	if (id === "zoom-in" || id === "zoom-out" || id === "zoom-reset") {
		zoom(id === "zoom-in" ? "in" : id === "zoom-out" ? "out" : "reset");
	} else if (
		id === "cut" ||
		id === "copy" ||
		id === "paste" ||
		id === "select-all"
	) {
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
	} else if (document.querySelector("dialog[open]") !== null) {
		return;
	} else if (id === "new-link") {
		openAdd(null);
	} else if (id === "new-collection") {
		void newFolder(current?.here ?? null);
	} else if (id === "open") {
		send({ kind: "open", id: null });
	} else if (id === "back" || id === "forward") {
		send({ kind: id });
	} else if (id === "show-in-folder") {
		send({ kind: "showInFolder", id: null });
	} else if (id === "empty-trash" && (current?.trash ?? 0) > 0) {
		void confirmEmptying();
	} else {
		const row = focusedRow();
		if (id === "edit-bookmark" && row?.kind === "shortcut" && !trashShown())
			edit(row.item);
		else if (id === "copy-url" && row?.kind === "shortcut" && !trashShown())
			void copy(row.item.link);
		else if (id === "rename-folder" && row?.kind === "folder" && !trashShown())
			renameFolder(row.folder.id, row.folder.name);
		else if (id === "mark-folder" && row?.kind === "folder" && !trashShown())
			markFolder(row.folder.id, row.folder.name, row.folder.mark);
		else if (id === "move-folder" && row?.kind === "folder" && !trashShown())
			moving.open(row.folder.id, row.folder.name);
		else if (id === "trash-selection" || (id === "put-back" && trashShown())) {
			const target: Target | null =
				(current?.selection.count ?? 0) > 0
					? { kind: "selection" }
					: row?.kind === "shortcut"
						? { kind: "ids", ids: [row.item.id] }
						: row?.kind === "folder"
							? { kind: "ids", ids: [row.folder.id] }
							: null;
			if (target === null) return;
			if (id === "put-back") send({ kind: "putBack", target });
			else if (trashShown())
				void confirmDeletion(
					target,
					current?.selection.count || 1,
					(current?.selection.count ?? 0) > 0
						? (current?.selection.folders ?? 0)
						: row?.kind === "folder"
							? 1
							: 0,
				);
			else send({ kind: "trash", target });
		}
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
function resize(): void {
	const compactWidth = 53.125;
	document.body.classList.toggle(
		"compact",
		document.documentElement.clientWidth <
			compactWidth *
				Number.parseFloat(getComputedStyle(document.documentElement).fontSize),
	);
	if (applyMetrics() && current !== null) {
		drawnListing = null;
		draw(current, false, viewportPins());
	} else {
		showListing();
	}
}

window.addEventListener("resize", resize);
const metrics = new ResizeObserver(resize);
metrics.observe(find("row-metrics", HTMLElement));
metrics.observe(find("heading-metrics", HTMLElement));

window.addEventListener("focus", refresh);
void listen(SUMMONED, () => {
	if (document.querySelector("dialog[open]") === null) content.focus();
});
void listen(ICONED, refresh);
void listenText(COMMAND, menuCommand);
void listenProgress(PROGRESS, showProgress);

send({ kind: "show" });
