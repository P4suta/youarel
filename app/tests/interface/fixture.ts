import { resolve } from "node:path";
import { expect, type Page } from "@playwright/test";
import type { Choice, Intent, Item, RowView, View } from "../../ui/types.js";

const ORIGIN = "http://youarel.test";
const SAVE_ERROR = "The library could not save the change. Try again.";
const UI = resolve(import.meta.dirname, "../../ui");
const ASSETS = new Set([
	"index.html",
	"style.css",
	"main.js",
	"dom.js",
	"draw.js",
	"types.js",
	"rows.js",
	"sheets.js",
	"picker.js",
	"url.js",
]);
export const BOOKMARK: Item = {
	id: "00000000000000000000000000000001",
	title: "Reference notes",
	link: "https://example.com/reference",
	url: {
		scheme: "https://",
		sub: "",
		name: "example",
		suffix: ".com",
		port: "",
		rest: "/reference",
	},
	icon: null,
	collection: null,
	collectionId: null,
	alias: "ref",
	note: "Saved reference material",
	added: 1_700_000_000_000,
	opened: null,
	visits: 0,
};
const COLLECTION = {
	id: "00000000000000000000000000000002",
	name: "Research",
	mark: "",
	count: 0,
};
const ROWS: readonly RowView[] = [
	{
		kind: "folder",
		folder: COLLECTION,
		origin: null,
		selected: false,
		cut: false,
	},
	{
		kind: "shortcut",
		item: BOOKMARK,
		selected: false,
		cut: false,
	},
];
export const INITIAL: View = {
	cut: 0,
	scope: { kind: "all" },
	trash: 0,
	title: "Bookmarks",
	count: 1,
	listing: {
		identity: "all",
		anchor: null,
		key: "initial",
		total: ROWS.length,
		first: 0,
		rows: ROWS,
		focus: null,
		reveal: null,
		fresh: true,
		folders: 1,
		groups: [],
	},
	focus: null,
	selection: { count: 0, folders: 0 },
	notice: null,
	here: null,
	trail: [],
	back: false,
	forward: false,
	undo: null,
	redo: null,
};

export interface Call {
	readonly command: string;
	readonly args: CommandArgs;
}

interface CommandArgs {
	readonly [name: string]: unknown;
	readonly intent?: unknown;
	readonly query?: unknown;
	readonly link?: unknown;
}

export interface FixtureOptions {
	readonly refuse?: string;
	readonly refuseIntent?: Intent["kind"];
	readonly errorView?: boolean;
	readonly notice?: View["notice"];
	readonly view?: View;
	readonly clipboard?: Promise<string | null>;
	readonly normalizedLinks?: Readonly<
		Record<string, string | null | Promise<string | null>>
	>;
	readonly choices?: readonly Choice[];
	readonly icons?: Readonly<Record<string, string>>;
}

export async function newBookmark(page: Page): Promise<void> {
	await page.keyboard.press("ControlOrMeta+n");
	await expect(
		page.getByRole("dialog", { name: "Add a link", exact: true }),
	).toBeVisible();
}

export async function rowMenu(page: Page, name: string): Promise<void> {
	await page
		.locator("#content")
		.getByText(name, { exact: true })
		.click({ button: "right" });
}

declare global {
	interface Window {
		youarelTestInvoke(command: string, args: CommandArgs): Promise<unknown>;
	}
}

export async function library(
	page: Page,
	options: FixtureOptions = {},
): Promise<Call[]> {
	const calls: Call[] = [];
	let view: View = {
		...(options.view ?? INITIAL),
		notice: options.notice ?? null,
	};
	const root = view;
	let anchor: string | null = null;
	await page.exposeFunction(
		"youarelTestInvoke",
		async (command: string, args: CommandArgs): Promise<unknown> => {
			calls.push({ command, args });
			if (command === options.refuse) {
				if (options.errorView === true)
					return {
						...view,
						notice: {
							text: SAVE_ERROR,
							error: true,
						},
					};
				throw new Error(SAVE_ERROR);
			}
			if (command === "choices") {
				const choices = options.choices ?? [
					{ id: COLLECTION.id, path: COLLECTION.name },
				];
				const query = typeof args.query === "string" ? args.query : "";
				return choices.filter((choice) =>
					choice.path.toLowerCase().includes(query.toLowerCase()),
				);
			}
			if (command === "clipboard") return options.clipboard ?? null;
			if (command === "normalize_link") {
				const link = typeof args.link === "string" ? args.link : "";
				if (
					options.normalizedLinks !== undefined &&
					link in options.normalizedLinks
				)
					return options.normalizedLinks[link];
				try {
					const url = new URL(link);
					return (url.protocol === "https:" || url.protocol === "http:") &&
						url.hostname !== ""
						? url.href
						: null;
				} catch (_invalidAddress: unknown) {
					return null;
				}
			}
			if (command === "title") return null;
			if (command === "icons") return options.icons ?? {};
			if (command === "emoji")
				return [
					{ label: "Objects", choices: [{ emoji: "📚", name: "books" }] },
				];
			if (command === "history_menu" || command === "copy") return null;
			if (command === "prepare_drag")
				return {
					token: "00000000000000000000000000000099",
					count: 1,
					links: BOOKMARK.link,
				};
			if (command === "page")
				return {
					key: view.listing.key,
					total: view.listing.total,
					first: 0,
					rows: view.listing.rows,
				};
			const intent = args.intent;
			if (
				command === "act" &&
				typeof intent === "object" &&
				intent !== null &&
				"kind" in intent
			) {
				if (intent.kind === options.refuseIntent) {
					if (options.errorView === true)
						return {
							...view,
							notice: {
								text: SAVE_ERROR,
								error: true,
							},
						};
					throw new Error(SAVE_ERROR);
				}
				if (intent.kind !== "show") view = { ...view, notice: null };
				if (intent.kind === "cancel") {
					view = {
						...view,
						selection: { count: 0, folders: 0 },
						listing: {
							...view.listing,
							rows: view.listing.rows.map((row) =>
								row.kind === "gone" ? row : { ...row, selected: false },
							),
						},
					};
				}
				if (
					intent.kind === "scope" &&
					"scope" in intent &&
					typeof intent.scope === "object" &&
					intent.scope !== null &&
					"kind" in intent.scope
				) {
					if (intent.scope.kind === "trash")
						view = {
							...view,
							scope: { kind: "trash" },
							trail: [],
							here: null,
							listing: {
								...view.listing,
								identity: "trash",
								key: "trash",
								rows: [],
								total: 0,
								folders: 0,
							},
						};
					else if (intent.scope.kind === "all")
						view = { ...root, listing: { ...root.listing, key: "root" } };
				}
				if (
					intent.kind === "select" &&
					"id" in intent &&
					typeof intent.id === "string"
				) {
					const id = intent.id;
					const how = "extend" in intent ? intent.extend : "replace";
					const selectedPlace = view.listing.rows.findIndex(
						(row) =>
							row.kind !== "gone" &&
							(row.kind === "folder" ? row.folder.id : row.item.id) === id,
					);
					const anchorPlace = view.listing.rows.findIndex(
						(row) =>
							row.kind !== "gone" &&
							(row.kind === "folder" ? row.folder.id : row.item.id) === anchor,
					);
					if (how !== "range") anchor = id;
					const rows = view.listing.rows.map(
						(row, place): RowView =>
							row.kind === "gone"
								? row
								: {
										...row,
										selected:
											how === "range" && anchorPlace >= 0
												? place >= Math.min(anchorPlace, selectedPlace) &&
													place <= Math.max(anchorPlace, selectedPlace)
												: (row.kind === "shortcut"
															? row.item.id
															: row.folder.id) === id
													? how !== "toggle" || !row.selected
													: how === "toggle" && row.selected,
									},
					);
					view = {
						...view,
						focus: id,
						selection: {
							count: rows.filter((row) => row.kind !== "gone" && row.selected)
								.length,
							folders: rows.filter(
								(row) => row.kind === "folder" && row.selected,
							).length,
						},
						listing: { ...view.listing, rows },
					};
				}
			}
			return view;
		},
	);
	await page.addInitScript((): void => {
		Object.defineProperty(window, "__TAURI__", {
			value: {
				core: {
					invoke: (command: string, args: CommandArgs = {}): Promise<unknown> =>
						window.youarelTestInvoke(command, args),
				},
				event: {
					listen: async (
						event: string,
						handler: (raised: { readonly payload: unknown }) => void,
					): Promise<() => void> => {
						const forward = (raised: Event): void => {
							if (raised instanceof CustomEvent) {
								const payload: unknown = raised.detail;
								handler({ payload });
							}
						};
						window.addEventListener(`youarel:${event}`, forward);
						return (): void =>
							window.removeEventListener(`youarel:${event}`, forward);
					},
				},
			},
		});
	});
	await page.route(`${ORIGIN}/**`, async (route): Promise<void> => {
		const name =
			new URL(route.request().url()).pathname.slice(1) || "index.html";
		if (!ASSETS.has(name)) {
			await route.fulfill({ status: 404 });
			return;
		}
		await route.fulfill({ path: resolve(UI, name) });
	});
	await page.goto(ORIGIN);
	await expect(page.locator("#content > *")).not.toHaveCount(0);
	return calls;
}
