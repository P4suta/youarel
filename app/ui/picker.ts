import { el, icon } from "./dom.js";
import type { Choice } from "./types.js";

const NATIVE_CHOICES = 8;
const PICK_DELAY_MS = 150;
const POPUP_GAP = 4;
const POPUP_HEIGHT_REM = 16;

export type PickCollections = (
	query: string,
	first: number,
) => Promise<readonly Choice[]>;

function option(label: string, value: string): HTMLOptionElement {
	const made = el("option", "", label);
	made.value = value;
	return made;
}

export class Picker {
	private chosen: Choice | null = null;
	private choices: readonly (Choice | null)[] = [];
	private initial: readonly Choice[] = [];
	private readonly popup = el("div", "picker-popup");
	private readonly list = el("div", "picker-options");
	private readonly status = el("p", "hint picker-status");
	private readonly toggle = el("button", "picker-toggle");
	private readonly arrow = icon("chevron", "glyph picker-arrow");
	private readonly label: HTMLLabelElement;
	private active = 0;
	private asked = 0;
	private timer: number | undefined;
	private composing = false;

	constructor(
		private readonly search: HTMLInputElement,
		private readonly select: HTMLSelectElement,
		private readonly pick: PickCollections,
		private readonly none = "Bookmarks",
	) {
		const label = select.labels?.[0];
		const holder = select.parentElement;
		if (label === undefined || holder === null)
			throw new Error("A folder picker needs its label and container.");
		this.label = label;
		label.id = `${select.id}-label`;
		this.list.id = `${select.id}-options`;
		this.list.setAttribute("role", "listbox");
		this.list.setAttribute("aria-labelledby", label.id);
		this.status.id = `${select.id}-status`;
		this.status.setAttribute("role", "status");
		search.setAttribute("role", "combobox");
		search.setAttribute("aria-controls", this.list.id);
		search.setAttribute("aria-autocomplete", "list");
		search.setAttribute("aria-expanded", "false");
		search.setAttribute(
			"aria-describedby",
			`${select.getAttribute("aria-describedby") ?? ""} ${this.status.id}`.trim(),
		);
		search.removeAttribute("aria-label");
		this.toggle.type = "button";
		this.toggle.tabIndex = -1;
		this.toggle.setAttribute("aria-label", "Show folders");
		this.toggle.append(icon("chevron"));
		this.toggle.hidden = true;
		this.popup.hidden = true;
		this.popup.append(this.list, this.status);
		holder.append(this.arrow, this.toggle, this.popup);
		select.addEventListener("change", () => {
			this.chosen =
				select.value === ""
					? null
					: {
							id: select.value,
							path: select.selectedOptions[0]?.textContent ?? "",
						};
		});
		search.addEventListener("focus", () => search.select());
		search.addEventListener("click", () => {
			if (this.popup.hidden) this.expand();
		});
		search.addEventListener("input", () => {
			if (!this.composing) this.filter();
		});
		search.addEventListener("compositionstart", () => {
			this.composing = true;
		});
		search.addEventListener("compositionend", () => {
			this.composing = false;
			this.filter();
		});
		search.addEventListener("blur", () => this.close());
		search.addEventListener("keydown", (event) => this.key(event));
		this.toggle.addEventListener("pointerdown", (event) =>
			event.preventDefault(),
		);
		this.toggle.addEventListener("click", () => {
			search.focus();
			if (this.popup.hidden) this.expand();
			else this.close();
		});
		window.addEventListener("resize", () => this.position());
		search.closest("dialog")?.addEventListener("scroll", () => this.position());
	}

	async open(chosen: Choice | null): Promise<void> {
		window.clearTimeout(this.timer);
		this.chosen = chosen;
		this.composing = false;
		this.close();
		this.asked += 1;
		const asked = this.asked;
		this.select.replaceChildren(
			option(this.none, ""),
			...(chosen === null ? [] : [option(chosen.path, chosen.id)]),
		);
		this.select.value = chosen?.id ?? "";
		const found = await this.pick("", 0);
		if (asked !== this.asked) return;
		const choices =
			chosen !== null && !found.some((choice) => choice.id === chosen.id)
				? [chosen, ...found]
				: found;
		this.select.replaceChildren(
			option(this.none, ""),
			...choices.map((choice) => option(choice.path, choice.id)),
		);
		this.select.value = chosen?.id ?? "";
		const searchable = choices.length > NATIVE_CHOICES;
		const focused =
			this.select === document.activeElement ||
			this.search === document.activeElement;
		this.select.hidden = searchable;
		this.search.hidden = !searchable;
		this.toggle.hidden = !searchable;
		this.arrow.classList.toggle("hidden-arrow", searchable);
		this.label.htmlFor = searchable ? this.search.id : this.select.id;
		this.choices = [null, ...choices];
		this.initial = choices;
		this.draw();
		if (focused) (searchable ? this.search : this.select).focus();
	}

	get value(): string | null {
		return this.chosen?.id ?? null;
	}

	private expand(): void {
		this.popup.hidden = false;
		this.search.setAttribute("aria-expanded", "true");
		this.active = Math.max(
			0,
			this.choices.findIndex((choice) => choice?.id === this.chosen?.id),
		);
		this.draw();
		this.position();
	}

	private close(): void {
		this.asked += 1;
		window.clearTimeout(this.timer);
		this.popup.hidden = true;
		const chosen = this.chosen;
		this.choices = [
			null,
			...(chosen !== null &&
			!this.initial.some((choice) => choice.id === chosen.id)
				? [chosen]
				: []),
			...this.initial,
		];
		this.status.textContent = "Type to find a folder.";
		this.search.value = this.chosen?.path ?? this.none;
		this.search.setAttribute("aria-expanded", "false");
		this.search.removeAttribute("aria-activedescendant");
	}

	private filter(): void {
		this.asked += 1;
		const asked = this.asked;
		const query = this.search.value;
		window.clearTimeout(this.timer);
		this.expand();
		this.search.removeAttribute("aria-activedescendant");
		this.choices = [];
		this.list.replaceChildren();
		this.status.textContent = "Finding folders…";
		this.timer = window.setTimeout(() => {
			void this.pick(query, 0)
				.then((found) => {
					if (asked !== this.asked) return;
					this.choices = [null, ...found];
					this.active = found.length > 0 ? 1 : -1;
					this.status.textContent =
						found.length === 0
							? "No matching folders. Try another name."
							: "Select a result to change the folder.";
					this.draw();
					this.position();
				})
				.catch(() => {
					if (asked === this.asked)
						this.status.textContent =
							"Folders could not be loaded. Retry the search.";
				});
		}, PICK_DELAY_MS);
	}

	private draw(): void {
		this.list.replaceChildren(
			...this.choices.map((choice, index) => {
				const item = el("div", "picker-option", choice?.path ?? this.none);
				item.id = `${this.list.id}-${index}`;
				item.setAttribute("role", "option");
				item.setAttribute("aria-selected", String(index === this.active));
				item.addEventListener("pointerdown", (event) => event.preventDefault());
				item.addEventListener("click", () => this.choose(index));
				return item;
			}),
		);
		this.highlight();
	}

	private highlight(): void {
		if (this.popup.hidden) return;
		for (const [index, item] of [...this.list.children].entries())
			item.setAttribute("aria-selected", String(index === this.active));
		const active = this.list.children[this.active];
		if (active instanceof HTMLElement) {
			this.search.setAttribute("aria-activedescendant", active.id);
			active.scrollIntoView({ block: "nearest" });
		} else {
			this.search.removeAttribute("aria-activedescendant");
		}
	}

	private choose(index: number): void {
		const choice = this.choices[index];
		if (choice === undefined) return;
		this.chosen = choice;
		this.close();
		this.search.focus();
	}

	private key(event: KeyboardEvent): void {
		if (event.isComposing || this.composing) return;
		if (
			event.altKey &&
			(event.key === "ArrowDown" || event.key === "ArrowUp")
		) {
			event.preventDefault();
			if (event.key === "ArrowUp") this.close();
			else this.expand();
			return;
		}
		if (event.metaKey || event.ctrlKey || event.altKey) return;
		if (event.key === "ArrowDown" || event.key === "ArrowUp") {
			event.preventDefault();
			if (this.popup.hidden) {
				this.expand();
				if (event.key === "ArrowUp") this.active = this.choices.length - 1;
			} else
				this.active = Math.max(
					0,
					Math.min(
						this.choices.length - 1,
						this.active + (event.key === "ArrowDown" ? 1 : -1),
					),
				);
			this.highlight();
		} else if (
			!this.popup.hidden &&
			(event.key === "Escape" || event.key === "Enter")
		) {
			event.preventDefault();
			event.stopPropagation();
			if (event.key === "Escape") this.close();
			else this.choose(this.active);
		} else if (event.key === "Tab") this.close();
	}

	private position(): void {
		if (this.popup.hidden) return;
		const bounds = this.search.getBoundingClientRect();
		const below = window.innerHeight - bounds.bottom - POPUP_GAP;
		const above = bounds.top - POPUP_GAP;
		const limit = Math.min(
			POPUP_HEIGHT_REM *
				Number.parseFloat(getComputedStyle(document.documentElement).fontSize),
			Math.max(above, below),
		);
		this.popup.style.width = `${bounds.width}px`;
		this.popup.style.maxHeight = `${Math.max(0, limit)}px`;
		this.popup.style.left = `${bounds.left}px`;
		this.popup.style.top = `${below >= above ? bounds.bottom + POPUP_GAP : Math.max(0, bounds.top - this.popup.getBoundingClientRect().height - POPUP_GAP)}px`;
	}
}
