// What crosses between the page and the library: the view the library draws, and the intents the page sends.
// The shapes mirror `app/src/view.rs` and `app/src/intent.rs`, which name every field the same way.

export type Key = "up" | "down" | "left" | "right" | "first" | "last";
export type Extend = "replace" | "toggle" | "range";

/** A place the window stands in: a scope of the library, or the trash. */
export type ScopeName =
	| { readonly kind: "all" }
	| { readonly kind: "unsorted" }
	| { readonly kind: "recent" }
	| { readonly kind: "collection"; readonly id: string }
	| { readonly kind: "trash" };

export interface UrlView {
	readonly scheme: string;
	readonly sub: string;
	readonly name: string;
	readonly suffix: string;
	readonly port: string;
	readonly rest: string;
}

export interface Item {
	readonly id: string;
	readonly title: string;
	readonly link: string;
	readonly url: UrlView;
	/** The name of the shortcut's icon, which the page asks the library to draw. */
	readonly icon: string | null;
	readonly collection: string | null;
	readonly collectionId: string | null;
	readonly alias: string;
	readonly note: string;
	readonly added: number;
	readonly opened: number | null;
	readonly visits: number;
}

export interface CollectionCount {
	readonly id: string;
	readonly name: string;
	/** What the folder shows in place of its glyph, or empty for the glyph. */
	readonly mark: string;
	readonly count: number;
}

/** A heading over the rows of a listing: its label, the place of the first row under it, and how many rows follow under it. */
export interface Group {
	readonly label: string;
	readonly place: number;
	readonly count: number;
}

/** One row of a listing: a folder, a shortcut, or a row whose entry went while the page was read. */
export type RowView =
	| {
			readonly kind: "folder";
			readonly folder: CollectionCount;
			/** The path of the folder it came from while it waits in the trash, or null. */
			readonly origin: string | null;
			readonly selected: boolean;
			readonly cut: boolean;
	  }
	| {
			readonly kind: "shortcut";
			readonly item: Item;
			readonly selected: boolean;
			readonly cut: boolean;
	  }
	| { readonly kind: "gone" };

/** The row used to preserve the list's scroll position. */
export interface AnchoredRow {
	readonly id: string;
	readonly place: number;
}

export interface ListingView {
	readonly identity: string;
	readonly anchor: AnchoredRow | null;
	/** What tells the listing apart in the state of the store it was read in. */
	readonly key: string;
	readonly total: number;
	/** The place of the first of the rows. */
	readonly first: number;
	readonly rows: readonly RowView[];
	/** The place of the focused row, when the listing holds it. */
	readonly focus: number | null;
	/** The place of a row the page brings into sight, after an intent that moved the focus to it. */
	readonly reveal: number | null;
	/** Whether the listing is another than the one shown before, so the page starts from its top. */
	readonly fresh: boolean;
	/** How many of the first rows are folders. */
	readonly folders: number;
	readonly groups: readonly Group[];
	/** Whether the rows form the list's outline, whose folders open in place. */
}

/** Rows of the listing from a place on, which the page asks for as a person scrolls. */
export interface RowsView {
	readonly key: string;
	readonly total: number;
	readonly first: number;
	readonly rows: readonly RowView[];
}

/** What a person selected, counted: how many rows, and how many of them are folders. */
export interface SelectionView {
	readonly count: number;
	readonly folders: number;
}

/** How far a move or a removal came. */
export interface Progress {
	readonly done: number;
	readonly total: number;
	readonly text: string;
}

/** What a move or a removal acts on: shortcuts and folders by their identifiers, or the selection as the library holds it. */
export type Target =
	| { readonly kind: "ids"; readonly ids: readonly string[] }
	| { readonly kind: "selection" }
	| { readonly kind: "drag"; readonly token: string };

export interface DragMaterial {
	readonly token: string;
	readonly links: string;
	readonly count: number;
}

export interface Choice {
	readonly id: string;
	readonly path: string;
}

export interface Notice {
	readonly text: string;
	readonly error: boolean;
}

export interface View {
	readonly cut: number;
	readonly scope: ScopeName;
	readonly trash: number;
	readonly title: string;
	readonly count: number;
	readonly listing: ListingView;
	readonly focus: string | null;
	readonly selection: SelectionView;
	readonly notice: Notice | null;
	/** The folder a new folder or a new link goes into, with its path, or null for the top. */
	readonly here: Choice | null;
	/** The collection trail from the root to the current folder. */
	readonly trail: readonly Step[];
	readonly back: boolean;
	readonly forward: boolean;
	readonly undo: string | null;
	readonly redo: string | null;
}

/** An emoji the mark picker offers, with the name its shortcode writes. */
export interface EmojiChoice {
	readonly emoji: string;
	readonly name: string;
}

/** A heading of the mark picker with the emoji under it. */
export interface EmojiGroup {
	readonly label: string;
	readonly choices: readonly EmojiChoice[];
}

export interface Step {
	readonly id: string;
	readonly name: string;
	/** The column the folder sits in, from the left. */
	readonly depth: number;
}

export interface Fields {
	readonly id: string;
	readonly title: string;
	readonly alias: string;
	readonly note: string;
	readonly collection: string | null;
}

export type Intent =
	| {
			readonly kind:
				| "show"
				| "undo"
				| "redo"
				| "cut"
				| "cancelCut"
				| "cancel"
				| "back"
				| "forward"
				| "selectAll"
				| "emptyTrash";
	  }
	| { readonly kind: "paste"; readonly text: string }
	| { readonly kind: "endDrag"; readonly token: string }
	| { readonly kind: "spring"; readonly token: string; readonly folder: string }
	| { readonly kind: "scope"; readonly scope: ScopeName }
	| { readonly kind: "showInFolder"; readonly id: string | null }
	| { readonly kind: "type"; readonly key: string }
	| {
			readonly kind: "select";
			readonly id: string;
			readonly extend: Extend;
			readonly place: number | null;
	  }
	| {
			readonly kind: "markCollection";
			readonly id: string;
			readonly mark: string;
	  }
	| {
			readonly kind: "step";
			readonly key: Key;
			readonly across: number;
			readonly extend: boolean;
	  }
	| { readonly kind: "open"; readonly id: string | null }
	| { readonly kind: "edit"; readonly fields: Fields }
	| { readonly kind: "retitle"; readonly id: string; readonly title: string }
	| {
			readonly kind: "file";
			readonly target: Target;
			readonly collection: string | null;
	  }
	| { readonly kind: "trash" | "putBack" | "remove"; readonly target: Target }
	| {
			readonly kind: "newCollection";
			readonly name: string;
			readonly parent: string | null;
	  }
	| {
			readonly kind: "renameCollection";
			readonly id: string;
			readonly name: string;
	  };

interface Tauri {
	readonly core: {
		invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
	};
	readonly event: {
		listen(
			event: string,
			handler: (raised: { readonly payload: unknown }) => void,
		): Promise<() => void>;
	};
}

declare global {
	interface Window {
		readonly __TAURI__: Tauri;
	}
}

/** Calls a command of the library. */
export function invoke<T>(
	command: string,
	args: Record<string, unknown> = {},
): Promise<T> {
	return window.__TAURI__.core.invoke<T>(command, args);
}

/** Calls `handler` each time the library raises `event`. */
export function listen(
	event: string,
	handler: () => void,
): Promise<() => void> {
	return window.__TAURI__.event.listen(event, () => handler());
}

/** Calls `handler` with the text each time the library raises `event` with one, and ignores a payload that is no text. */
export function listenText(
	event: string,
	handler: (text: string) => void,
): Promise<() => void> {
	return window.__TAURI__.event.listen(event, (raised) => {
		if (typeof raised.payload === "string") {
			handler(raised.payload);
		}
	});
}

/** Whether a payload is the progress of a job. */
function isProgress(payload: unknown): payload is Progress {
	if (typeof payload !== "object" || payload === null) {
		return false;
	}
	if (!("done" in payload) || !("total" in payload) || !("text" in payload)) {
		return false;
	}
	return (
		typeof payload.done === "number" &&
		typeof payload.total === "number" &&
		typeof payload.text === "string"
	);
}

/** Calls `handler` with each progress the library raises under `event`, and ignores a payload of another shape. */
export function listenProgress(
	event: string,
	handler: (progress: Progress) => void,
): Promise<() => void> {
	return window.__TAURI__.event.listen(event, (raised) => {
		if (isProgress(raised.payload)) {
			handler(raised.payload);
		}
	});
}
