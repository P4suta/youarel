// The sheets: adding a link, editing a shortcut, asking for a name or a confirmation, and the menu of one shortcut.

import { containFocus, el, find, icon } from "./dom.js";
import { type PickCollections, Picker } from "./picker.js";
import type { Choice, EmojiGroup, Fields, Item } from "./types.js";

/** How long the add sheet waits after the last keystroke in the link before it reads the page's title. */
const TITLE_DELAY_MS = 600;

/** Closes a sheet from each of its buttons marked to close it. */
function closable(
	dialog: HTMLDialogElement,
	closed: () => void = () => undefined,
): void {
	containFocus(dialog);
	for (const close of dialog.querySelectorAll("[data-close]")) {
		close.addEventListener("click", () => dialog.close());
	}
	dialog.addEventListener("close", () => {
		if (dialog.open) return;
		closed();
		if (document.querySelector("dialog[open]") !== null) {
			return;
		}
		const active = document.activeElement;
		if (
			active !== null &&
			active !== document.body &&
			active !== document.documentElement &&
			!dialog.contains(active)
		)
			return;
		const opener = openers.get(dialog);
		const key = opener?.getAttribute("data-focus-key");
		const replacement =
			key === null || key === undefined
				? null
				: document.querySelector<HTMLElement>(
						`[data-focus-key="${CSS.escape(key)}"]`,
					);
		const target = opener?.isConnected === true ? opener : replacement;
		(target ?? find("content", HTMLElement)).focus({ preventScroll: true });
	});
}

const openers = new WeakMap<HTMLDialogElement, HTMLElement>();

function showSheet(dialog: HTMLDialogElement): void {
	const heading = dialog.querySelector("h2")?.textContent;
	if (heading === undefined || heading === null || heading === "")
		throw new Error("A sheet needs a visible title.");
	dialog.setAttribute("aria-label", heading);
	dialog.removeAttribute("aria-labelledby");
	if (document.activeElement instanceof HTMLElement) {
		openers.set(dialog, document.activeElement);
	}
	dialog.showModal();
}

export type FormResult =
	| { readonly ok: true }
	| { readonly ok: false; readonly message: string };

/** What the add sheet asks of the library. */
export interface Adding {
	clipboard(): Promise<string | null>;
	normalize(link: string): Promise<string | null>;
	title(link: string): Promise<string | null>;
	add(
		link: string,
		title: string,
		collection: string | null,
	): Promise<FormResult>;
}

/** The sheet that adds a link. */
export class AddSheet {
	private readonly dialog = find("add-sheet", HTMLDialogElement);
	private readonly form = find("add-form", HTMLFormElement);
	private readonly link = find("add-link", HTMLInputElement);
	private readonly title = find("add-title", HTMLInputElement);
	private parent: string | null = null;
	private readonly clipped = find("add-from-clipboard", HTMLElement);
	private readonly submit = find("add-submit", HTMLButtonElement);
	private readonly error = find("add-error", HTMLElement);
	private readonly linkError = find("add-link-error", HTMLElement);
	private readonly titleHelp = find("add-title-help", HTMLElement);
	private generation = 0;
	private busy = false;
	private typedTitle = false;
	private timer: number | undefined;
	/** The link whose title the sheet asked for last, so an older answer never overwrites a newer one. */
	private asked = "";

	constructor(private readonly adding: Adding) {
		closable(this.dialog);
		this.title.addEventListener("input", () => {
			this.typedTitle = this.title.value !== "";
		});
		this.link.addEventListener("input", () => {
			this.clipped.hidden = true;
			this.link.removeAttribute("aria-invalid");
			this.linkError.hidden = true;
			window.clearTimeout(this.timer);
			this.timer = window.setTimeout(
				() => void this.readTitle(),
				TITLE_DELAY_MS,
			);
		});
		this.form.addEventListener("submit", (event) => {
			event.preventDefault();
			void this.send();
		});
	}

	/** Opens the sheet over a collection with a link dropped on the window, or the one on the clipboard. */
	async open(collection: Choice | null, dropped: string | null): Promise<void> {
		this.generation += 1;
		const generation = this.generation;
		this.parent = collection?.id ?? null;
		this.link.value = dropped ?? "";
		this.title.value = "";
		this.typedTitle = false;
		this.clipped.hidden = true;
		this.busy = false;
		this.submit.removeAttribute("aria-disabled");
		this.form.removeAttribute("aria-busy");
		this.submit.textContent = "Add";
		this.error.hidden = true;
		this.linkError.hidden = true;
		this.link.removeAttribute("aria-invalid");
		this.titleHelp.textContent =
			"Optional. The page title is filled in when available.";
		this.asked = "";
		showSheet(this.dialog);
		this.link.focus();
		if (dropped === null) {
			try {
				const clipped = await this.adding.clipboard();
				if (!this.dialog.open || generation !== this.generation) {
					return;
				}
				if (
					clipped !== null &&
					this.link.value === "" &&
					document.activeElement === this.link
				) {
					this.link.value = clipped;
					this.clipped.hidden = false;
					this.link.select();
				}
			} catch (_unavailableClipboard: unknown) {
				this.clipped.hidden = true;
			}
		}
		await this.readTitle();
	}

	private async readTitle(): Promise<void> {
		const link = this.link.value.trim();
		if (!this.dialog.open || link === "" || this.typedTitle) {
			return;
		}
		this.asked = link;
		const generation = this.generation;
		try {
			const normalized = await this.adding.normalize(link);
			if (
				!this.current(link, generation) ||
				this.asked !== link ||
				this.typedTitle
			)
				return;
			if (normalized === null) {
				this.titleHelp.textContent =
					"Optional. The page title is filled in when available.";
				return;
			}
			this.titleHelp.textContent = "Reading the page title…";
			const title = await this.adding.title(normalized);
			if (
				this.current(link, generation) &&
				this.asked === link &&
				!this.typedTitle
			) {
				this.title.value = title ?? "";
				this.titleHelp.textContent =
					title === null
						? "Optional. A title is unavailable. Enter a title or use the URL."
						: "The page title is filled in. You can edit the title.";
			}
		} catch (_unavailableTitle: unknown) {
			if (this.current(link, generation) && !this.typedTitle) {
				this.titleHelp.textContent =
					"The page title could not be read. Enter a title or use the URL.";
			}
		}
	}

	private current(link: string, generation: number): boolean {
		return (
			this.dialog.open &&
			generation === this.generation &&
			this.link.value.trim() === link
		);
	}

	private showError(message: string): void {
		this.error.textContent = `The bookmark could not be added. ${message}`;
		this.error.hidden = false;
		this.error.focus();
	}

	private async send(): Promise<void> {
		if (this.busy) {
			return;
		}
		const link = this.link.value.trim();
		const generation = this.generation;
		this.busy = true;
		this.error.hidden = true;
		this.submit.setAttribute("aria-disabled", "true");
		this.form.setAttribute("aria-busy", "true");
		this.submit.textContent = "Adding…";
		try {
			const normalized = await this.adding.normalize(link);
			if (!this.current(link, generation)) return;
			if (normalized === null) {
				this.linkError.textContent =
					"Enter a web address such as google.com or https://google.com. Use http:// for an HTTP site.";
				this.linkError.hidden = false;
				this.link.setAttribute("aria-invalid", "true");
				this.link.focus();
				return;
			}
			const added = await this.adding.add(
				normalized,
				this.title.value,
				this.parent,
			);
			if (generation === this.generation && this.dialog.open) {
				if (added.ok) {
					this.dialog.close();
				} else {
					this.showError(added.message);
				}
			}
		} catch (error: unknown) {
			if (generation === this.generation && this.dialog.open) {
				this.showError(String(error));
			}
		} finally {
			if (generation === this.generation) {
				this.busy = false;
				this.submit.removeAttribute("aria-disabled");
				this.form.removeAttribute("aria-busy");
				this.submit.textContent = "Add";
			}
		}
	}
}

/** The sheet that edits every field of a shortcut at once. */
export class EditSheet {
	private readonly dialog = find("edit-sheet", HTMLDialogElement);
	private readonly form = find("edit-form", HTMLFormElement);
	private readonly title = find("edit-title", HTMLInputElement);
	private readonly link = find("edit-link", HTMLElement);
	private readonly alias = find("edit-alias", HTMLInputElement);
	private readonly collection: Picker;
	private readonly note = find("edit-note", HTMLTextAreaElement);
	private readonly remove = find("edit-remove", HTMLButtonElement);
	private readonly submit = find("edit-submit", HTMLButtonElement);
	private readonly error = find("edit-error", HTMLElement);
	private readonly titleError = find("edit-title-error", HTMLElement);
	private busy = false;
	private generation = 0;
	private id = "";

	/** The sheet that saves every field with `save` and moves the shortcut to the trash with `trashed`. */
	constructor(
		private readonly save: (fields: Fields) => Promise<FormResult>,
		trashed: (id: string) => void,
		pick: PickCollections,
	) {
		this.collection = new Picker(
			find("edit-collection-search", HTMLInputElement),
			find("edit-collection", HTMLSelectElement),
			pick,
		);
		closable(this.dialog);
		this.form.addEventListener("submit", (event) => {
			event.preventDefault();
			void this.send();
		});
		this.remove.addEventListener("click", () => {
			this.dialog.close();
			trashed(this.id);
		});
	}

	private async send(): Promise<void> {
		if (this.busy) {
			return;
		}
		this.titleError.hidden = this.title.value.trim() !== "";
		this.title.setAttribute("aria-invalid", String(!this.titleError.hidden));
		if (!this.titleError.hidden) {
			this.titleError.textContent = "Enter a title for the bookmark.";
			this.title.focus();
			return;
		}
		this.busy = true;
		const generation = this.generation;
		this.error.hidden = true;
		this.submit.setAttribute("aria-disabled", "true");
		this.form.setAttribute("aria-busy", "true");
		this.submit.textContent = "Saving…";
		try {
			const result = await this.save({
				id: this.id,
				title: this.title.value,
				alias: this.alias.value.trim(),
				note: this.note.value,
				collection: this.collection.value,
			});
			if (generation !== this.generation || !this.dialog.open) return;
			if (result.ok) {
				this.dialog.close();
			} else {
				this.error.textContent = `The bookmark could not be saved. ${result.message}`;
				this.error.hidden = false;
				this.error.focus();
			}
		} catch (error: unknown) {
			if (generation !== this.generation || !this.dialog.open) return;
			this.error.textContent = `The bookmark could not be saved. ${String(error)}`;
			this.error.hidden = false;
			this.error.focus();
		} finally {
			if (generation === this.generation) {
				this.busy = false;
				this.submit.removeAttribute("aria-disabled");
				this.form.removeAttribute("aria-busy");
				this.submit.textContent = "Save";
			}
		}
	}

	/** Opens the sheet over a shortcut. */
	open(item: Item): void {
		this.generation += 1;
		const generation = this.generation;
		this.busy = false;
		this.submit.removeAttribute("aria-disabled");
		this.form.removeAttribute("aria-busy");
		this.submit.textContent = "Save";
		this.id = item.id;
		this.title.value = item.title;
		this.link.textContent = item.link;
		find("edit-added", HTMLElement).textContent =
			`Added ${new Intl.DateTimeFormat("en", { dateStyle: "long" }).format(new Date(item.added))}`;
		this.alias.value = item.alias;
		const home =
			item.collectionId === null || item.collection === null
				? null
				: { id: item.collectionId, path: item.collection };
		void this.collection.open(home).catch((error: unknown) => {
			if (generation !== this.generation || !this.dialog.open) return;
			this.error.textContent = `Folders could not be loaded. ${String(error)}`;
			this.error.hidden = false;
		});
		this.note.value = item.note;
		this.error.hidden = true;
		this.titleError.hidden = true;
		this.title.removeAttribute("aria-invalid");
		showSheet(this.dialog);
		this.title.select();
	}
}

/** What the ask sheet asks for. */
export interface Question {
	readonly title: string;
	readonly text: string;
	readonly action: string;
	/** The answer it starts from, or none for a confirmation without a field. */
	readonly answer: string | null;
	readonly secret?: boolean;
	readonly save?: (answer: string) => Promise<FormResult>;
}

export class MoveSheet {
	private readonly dialog = find("move-sheet", HTMLDialogElement);
	private readonly form = find("move-form", HTMLFormElement);
	private readonly error = find("move-error", HTMLElement);
	private readonly submit = find("move-submit", HTMLButtonElement);
	private readonly collection: Picker;
	private id = "";
	private busy = false;
	private generation = 0;

	constructor(
		pick: PickCollections,
		private readonly move: (
			id: string,
			collection: string | null,
		) => Promise<FormResult>,
	) {
		this.collection = new Picker(
			find("move-collection-search", HTMLInputElement),
			find("move-collection", HTMLSelectElement),
			pick,
			"Bookmarks",
		);
		closable(this.dialog);
		this.form.addEventListener("submit", (event) => {
			event.preventDefault();
			void this.send();
		});
	}

	open(id: string, name: string): void {
		this.generation += 1;
		const generation = this.generation;
		this.busy = false;
		this.submit.removeAttribute("aria-disabled");
		this.form.removeAttribute("aria-busy");
		this.id = id;
		this.error.hidden = true;
		find("move-name", HTMLElement).textContent = name;
		void this.collection.open(null).catch((error: unknown) => {
			if (generation === this.generation && this.dialog.open)
				this.showError(String(error));
		});
		showSheet(this.dialog);
		find("move-collection", HTMLSelectElement).focus();
	}

	private showError(message: string): void {
		this.error.textContent = `The folder could not be moved. ${message}`;
		this.error.hidden = false;
		this.error.focus();
	}

	private async send(): Promise<void> {
		if (this.busy) {
			return;
		}
		this.busy = true;
		const generation = this.generation;
		this.form.setAttribute("aria-busy", "true");
		this.submit.setAttribute("aria-disabled", "true");
		try {
			const result = await this.move(this.id, this.collection.value);
			if (generation !== this.generation || !this.dialog.open) return;
			if (result.ok) {
				this.dialog.close();
			} else {
				this.showError(result.message);
			}
		} catch (error: unknown) {
			if (generation === this.generation && this.dialog.open)
				this.showError(String(error));
		} finally {
			if (generation === this.generation) {
				this.busy = false;
				this.form.removeAttribute("aria-busy");
				this.submit.removeAttribute("aria-disabled");
			}
		}
	}
}

/** The sheet that asks for a name or a confirmation. */
export class AskSheet {
	private readonly dialog = find("ask-sheet", HTMLDialogElement);
	private readonly form = find("ask-form", HTMLFormElement);
	private readonly title = find("ask-title", HTMLElement);
	private readonly text = find("ask-text", HTMLElement);
	private readonly input = find("ask-input", HTMLInputElement);
	private readonly submit = find("ask-submit", HTMLButtonElement);
	private readonly error = find("ask-error", HTMLElement);
	private readonly nameError = find("ask-name-error", HTMLElement);
	private question: Question | null = null;
	private busy = false;
	private settle: ((answer: string | null) => void) | undefined;

	constructor() {
		closable(this.dialog, () => this.finish(null));
		this.form.addEventListener("submit", (event) => {
			event.preventDefault();
			void this.send();
		});
	}

	private finish(answer: string | null): void {
		const settle = this.settle;
		this.settle = undefined;
		this.question = null;
		this.input.value = "";
		if (this.dialog.open) {
			this.dialog.close();
		}
		settle?.(answer);
	}

	private async send(): Promise<void> {
		const question = this.question;
		if (question === null || this.busy) return;
		if (question.answer !== null && this.input.value.trim() === "") {
			this.nameError.textContent = "Enter a folder name.";
			this.nameError.hidden = false;
			this.input.setAttribute("aria-invalid", "true");
			this.input.focus();
			return;
		}
		this.nameError.hidden = true;
		this.input.removeAttribute("aria-invalid");
		if (question.save === undefined) {
			this.finish(this.input.value);
			return;
		}
		this.busy = true;
		this.submit.setAttribute("aria-disabled", "true");
		this.form.setAttribute("aria-busy", "true");
		try {
			const result = await question.save(this.input.value);
			if (question !== this.question || !this.dialog.open) return;
			if (result.ok) {
				this.finish(this.input.value);
			} else {
				this.error.textContent = `The folder could not be saved. ${result.message}`;
				this.error.hidden = false;
				this.error.focus();
			}
		} catch (error: unknown) {
			if (question === this.question && this.dialog.open) {
				this.error.textContent = `The folder could not be saved. ${String(error)}`;
				this.error.hidden = false;
				this.error.focus();
			}
		} finally {
			if (question === this.question) {
				this.busy = false;
				this.submit.removeAttribute("aria-disabled");
				this.form.removeAttribute("aria-busy");
			}
		}
	}

	/** Asks, and answers with the text, an empty text for a confirmation, or none when the person cancels. */
	ask(question: Question): Promise<string | null> {
		this.finish(null);
		this.question = question;
		this.busy = false;
		this.submit.removeAttribute("aria-disabled");
		this.form.removeAttribute("aria-busy");
		this.error.hidden = true;
		this.nameError.hidden = true;
		this.input.removeAttribute("aria-invalid");
		this.title.textContent = question.title;
		this.text.textContent = question.text;
		this.text.hidden = question.text === "";
		this.submit.textContent = question.action;
		this.submit.className = question.answer === null ? "danger" : "primary";
		this.input.hidden = question.answer === null;
		find("ask-field", HTMLElement).hidden = question.answer === null;
		find("ask-label", HTMLElement).hidden = question.answer === null;
		this.input.required = question.answer !== null;
		this.input.value = question.answer ?? "";
		this.input.type = question.secret === true ? "password" : "text";
		return new Promise((resolve) => {
			this.settle = resolve;
			showSheet(this.dialog);
			if (question.answer === null) {
				this.dialog.querySelector<HTMLButtonElement>("[data-close]")?.focus();
			} else {
				this.input.select();
			}
		});
	}
}

/** The emoji the picker shows for a search, as the library answers it. */
export type PickEmoji = (query: string) => Promise<readonly EmojiGroup[]>;

/** The sheet that gives a folder a mark: a search that takes `:name:`, the emoji under their headings, and a preview of the mark. */
export class MarkSheet {
	private readonly dialog = find("mark-sheet", HTMLDialogElement);
	private readonly form = find("mark-form", HTMLFormElement);
	private readonly title = find("mark-title", HTMLElement);
	private readonly preview = find("mark-preview", HTMLElement);
	private readonly input = find("mark-input", HTMLInputElement);
	private readonly grid = find("mark-grid", HTMLElement);
	private readonly clear = find("mark-clear", HTMLButtonElement);
	private settle: ((mark: string | null) => void) | undefined;
	/** The emoji picked last, which the preview shows and the sheet sets. */
	private chosen = "";
	/** The first emoji the search finds, which the return key sets when nothing was picked since. */
	private first = "";
	/** The search in flight, so a slower answer to an older one draws nothing. */
	private asked = 0;

	constructor(private readonly pick: PickEmoji) {
		closable(this.dialog, () => this.finish(null));
		this.form.addEventListener("submit", (event) => {
			event.preventDefault();
			this.finish(this.mark());
		});
		this.clear.addEventListener("click", () => this.finish(""));
		this.input.addEventListener("input", () => {
			// A typed emoji, such as one from the system's picker, is the mark itself; letters search.
			const typed = this.input.value.trim();
			this.chosen = /\P{ASCII}/u.test(typed) ? typed : "";
			this.show();
			void this.search(typed);
		});
	}

	/** The mark the sheet sets: the emoji picked, the first one found, or the text as typed, which the library reads `:name:` in. */
	private mark(): string {
		if (this.chosen !== "") {
			return this.chosen;
		}
		return this.first === "" ? this.input.value : this.first;
	}

	private show(): void {
		const shown = this.mark();
		const defaultIcon = shown === "" || /^[\x20-\x7e]*$/u.test(shown);
		this.preview.setAttribute("role", "img");
		this.preview.setAttribute(
			"aria-label",
			defaultIcon ? "Default folder icon" : shown,
		);
		this.preview.replaceChildren(defaultIcon ? icon("folder") : shown);
	}

	private async search(query: string): Promise<void> {
		this.asked += 1;
		const asked = this.asked;
		let groups: readonly EmojiGroup[];
		try {
			groups = await this.pick(query);
		} catch (_unavailableEmoji: unknown) {
			if (asked === this.asked && this.dialog.open)
				this.grid.replaceChildren(
					el("p", "hint", "Emoji could not be loaded. Try the search again."),
				);
			return;
		}
		if (asked !== this.asked) {
			return;
		}
		this.first = query === "" ? "" : (groups[0]?.choices[0]?.emoji ?? "");
		this.show();
		this.grid.replaceChildren(
			...groups.flatMap((group) => [
				el("h3", "mark-heading", group.label),
				...(group.choices.length === 0
					? [el("p", "hint", "No emoji has that name.")]
					: []),
				...group.choices.map((choice) => {
					const pick = el("button", "mark-choice", choice.emoji);
					pick.type = "button";
					pick.title = `:${choice.name}:`;
					pick.setAttribute("aria-label", choice.name.replaceAll("_", " "));
					pick.setAttribute(
						"aria-pressed",
						String(choice.emoji === this.chosen),
					);
					pick.addEventListener("click", () => {
						this.chosen = choice.emoji;
						for (const button of this.grid.querySelectorAll("button")) {
							button.setAttribute("aria-pressed", String(button === pick));
						}
						this.show();
					});
					pick.addEventListener("dblclick", () => this.finish(choice.emoji));
					return pick;
				}),
			]),
		);
	}

	private finish(mark: string | null): void {
		const settle = this.settle;
		this.settle = undefined;
		if (this.dialog.open) {
			this.dialog.close();
		}
		settle?.(mark);
	}

	/** Asks for a folder's mark, and answers with it, an empty one to remove it, or none when the person cancels. */
	open(name: string, mark: string): Promise<string | null> {
		this.finish(null);
		this.title.textContent = `Emoji for ${name}`;
		this.chosen = mark;
		this.first = "";
		this.input.value = "";
		this.clear.hidden = mark === "";
		this.show();
		void this.search("");
		return new Promise((resolve) => {
			this.settle = resolve;
			showSheet(this.dialog);
			this.input.focus();
		});
	}
}

/** A choice of a menu, which a check marks when it stands for something on or off, or a line between choices. */
export type MenuChoice =
	| {
			readonly label: string;
			readonly icon?: string;
			readonly danger?: boolean;
			readonly checked?: boolean;
			readonly act: () => void;
	  }
	| "line";

/** The menu a right click opens beside the pointer. */
export class Menu {
	private readonly menu = find("menu", HTMLElement);
	private trigger: HTMLElement | null = null;
	private typed = "";
	private lastTyped = 0;

	constructor() {
		document.addEventListener("click", (event) => {
			if (event.target instanceof Node && !this.menu.contains(event.target)) {
				this.close(false);
			}
		});
		window.addEventListener("blur", () => this.close(false));
		this.menu.addEventListener("keydown", (event) => this.key(event));
		window.addEventListener("resize", () => this.close());
	}

	get open(): boolean {
		return !this.menu.hidden;
	}

	close(restore = true): void {
		if (!this.open) {
			return;
		}
		this.menu.hidden = true;
		const key = this.trigger?.getAttribute("data-focus-key");
		const trigger =
			key === null || key === undefined
				? this.trigger
				: document.querySelector<HTMLElement>(
						`[data-focus-key="${CSS.escape(key)}"]`,
					);
		if (trigger?.hasAttribute("aria-haspopup") === true)
			trigger.setAttribute("aria-expanded", "false");
		if (restore) {
			(trigger?.isConnected === true
				? trigger
				: find("content", HTMLElement)
			).focus({ preventScroll: true });
		}
		this.trigger = null;
	}

	private key(event: KeyboardEvent): void {
		const items = [...this.menu.querySelectorAll<HTMLButtonElement>("button")];
		const active = document.activeElement;
		const current =
			active instanceof HTMLButtonElement ? items.indexOf(active) : -1;
		let next: number | undefined;
		if (event.key === "Escape" || event.key === "Tab") {
			this.close();
			if (event.key === "Escape") {
				event.preventDefault();
			}
		} else if (event.key === "ArrowDown") {
			next = (current + 1) % items.length;
		} else if (event.key === "ArrowUp") {
			next = (current + items.length - 1) % items.length;
		} else if (event.key === "Home") {
			next = 0;
		} else if (event.key === "End") {
			next = items.length - 1;
		} else if (
			event.key.length === 1 &&
			!event.metaKey &&
			!event.ctrlKey &&
			event.key !== " "
		) {
			const TYPEAHEAD_MS = 700;
			this.typed =
				(event.timeStamp - this.lastTyped > TYPEAHEAD_MS ? "" : this.typed) +
				event.key.toLowerCase();
			this.lastTyped = event.timeStamp;
			const found = items.findIndex(
				(item) =>
					item.textContent?.toLowerCase().startsWith(this.typed) === true,
			);
			if (found >= 0) {
				next = found;
			}
		}
		if (next !== undefined) {
			event.preventDefault();
			items[next]?.focus();
		}
		event.stopPropagation();
	}

	show(
		event: MouseEvent | KeyboardEvent,
		choices: readonly MenuChoice[],
	): void {
		event.preventDefault();
		event.stopPropagation();
		this.close(false);
		const owner =
			event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
		const active = document.activeElement;
		this.trigger =
			owner !== null && owner.tabIndex >= 0
				? owner
				: active instanceof HTMLElement &&
						(owner === null || owner.contains(active))
					? active
					: (owner?.querySelector<HTMLElement>("button, [tabindex='0']") ??
						null);
		if (this.trigger?.hasAttribute("aria-haspopup") === true)
			this.trigger.setAttribute("aria-expanded", "true");
		(document.querySelector("dialog[open]") ?? document.body).append(this.menu);
		this.typed = "";
		this.menu.replaceChildren(
			...choices.map((choice) => {
				if (choice === "line") {
					return el("hr");
				}
				const item = el(
					"button",
					choice.danger === true ? "danger-item" : "",
					choice.label,
				);
				item.type = "button";
				if (choice.icon !== undefined) item.prepend(icon(choice.icon));
				item.tabIndex = -1;
				if (choice.checked === undefined) {
					item.setAttribute("role", "menuitem");
				} else {
					item.setAttribute("role", "menuitemcheckbox");
					item.setAttribute("aria-checked", String(choice.checked));
				}
				item.addEventListener("click", () => {
					this.close();
					choice.act();
				});
				return item;
			}),
		);
		this.menu.hidden = false;
		const anchor = this.trigger?.getBoundingClientRect();
		const x = event instanceof MouseEvent ? event.clientX : (anchor?.left ?? 0);
		const y =
			event instanceof MouseEvent
				? event.clientY
				: this.trigger?.id === "content"
					? (anchor?.top ?? 0)
					: (anchor?.bottom ?? 0);
		const { width, height } = this.menu.getBoundingClientRect();
		this.menu.style.left = `${Math.max(0, Math.min(x, window.innerWidth - width))}px`;
		this.menu.style.top = `${Math.max(0, Math.min(y, window.innerHeight - height))}px`;
		const first = this.menu.querySelector<HTMLButtonElement>("button");
		if (first !== null) {
			first.tabIndex = 0;
			first.focus();
		}
	}
}
