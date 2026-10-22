// The sheets: adding a link, editing a shortcut, asking for a name or a confirmation, and the menu of one shortcut.

import { el, find, icon } from "./dom.js";
import type { Choice, EmojiGroup, Fields, Item } from "./types.js";

/** How long the add sheet waits after the last keystroke in the link before it reads the page's title. */
const TITLE_DELAY_MS = 600;
/** How long a collection picker waits after the last keystroke in its search before it asks the library. */
const PICK_DELAY_MS = 150;

/** The collections a search finds from a place on, as the library answers them, best first. */
export type PickCollections = (query: string, first: number) => Promise<readonly Choice[]>;

function option(label: string, value: string): HTMLOptionElement {
  const made = el("option", "", label);
  made.value = value;
  return made;
}

/** A collection picker: a search over a select of the collections it finds, which keeps the one chosen even when a search does not find it. */
export class Picker {
  private chosen: Choice | null = null;
  private timer: number | undefined;
  /** The search in flight, so a slower answer to an older one draws nothing. */
  private asked = 0;

  /** A picker over `search` and `select` that names no collection `none` and calls `changed` once a person picks another. */
  constructor(
    private readonly search: HTMLInputElement,
    private readonly select: HTMLSelectElement,
    private readonly pick: PickCollections,
    private readonly none = "No collection",
    changed: () => void = () => undefined,
  ) {
    search.addEventListener("input", () => {
      window.clearTimeout(this.timer);
      this.timer = window.setTimeout(() => void this.find(), PICK_DELAY_MS);
    });
    select.addEventListener("change", () => {
      const label = select.selectedOptions[0]?.textContent ?? "";
      this.chosen = select.value === "" ? null : { id: select.value, path: label };
      changed();
    });
  }

  /** Shows the collections from the first on, with `chosen` chosen. */
  async open(chosen: Choice | null): Promise<void> {
    this.chosen = chosen;
    this.search.value = "";
    await this.find();
  }

  private async find(): Promise<void> {
    this.asked += 1;
    const asked = this.asked;
    const found = await this.pick(this.search.value, 0);
    if (asked !== this.asked) {
      return;
    }
    const chosen = this.chosen;
    const options = found.map((choice) => option(choice.path, choice.id));
    if (chosen !== null && !found.some((choice) => choice.id === chosen.id)) {
      options.unshift(option(chosen.path, chosen.id));
    }
    this.select.replaceChildren(option(this.none, ""), ...options);
    this.select.value = chosen?.id ?? "";
  }

  /** The collection chosen, or null for none. */
  get value(): string | null {
    return this.select.value === "" ? null : this.select.value;
  }
}

/** Closes a sheet from each of its buttons marked to close it. */
function closable(dialog: HTMLDialogElement): void {
  for (const close of dialog.querySelectorAll("[data-close]")) {
    close.addEventListener("click", () => dialog.close());
  }
}

/** What the add sheet asks of the library. */
export interface Adding {
  clipboard(): Promise<string | null>;
  title(link: string): Promise<string | null>;
  add(link: string, title: string, collection: string | null): Promise<boolean>;
}

/** The sheet that adds a link. */
export class AddSheet {
  private readonly dialog = find("add-sheet", HTMLDialogElement);
  private readonly form = find("add-form", HTMLFormElement);
  private readonly link = find("add-link", HTMLInputElement);
  private readonly title = find("add-title", HTMLInputElement);
  private readonly collection: Picker;
  private readonly clipped = find("add-from-clipboard", HTMLElement);
  private readonly submit = find("add-submit", HTMLButtonElement);
  private typedTitle = false;
  private timer: number | undefined;
  /** The link whose title the sheet asked for last, so an older answer never overwrites a newer one. */
  private asked = "";

  constructor(
    private readonly adding: Adding,
    pick: PickCollections,
  ) {
    this.collection = new Picker(find("add-collection-search", HTMLInputElement), find("add-collection", HTMLSelectElement), pick);
    closable(this.dialog);
    this.title.addEventListener("input", () => {
      this.typedTitle = this.title.value !== "";
    });
    this.link.addEventListener("input", () => {
      this.clipped.hidden = true;
      window.clearTimeout(this.timer);
      this.timer = window.setTimeout(() => void this.readTitle(), TITLE_DELAY_MS);
    });
    this.form.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.send();
    });
  }

  /** Opens the sheet over a collection with a link dropped on the window, or the one on the clipboard. */
  async open(collection: Choice | null, dropped: string | null): Promise<void> {
    void this.collection.open(collection);
    this.link.value = dropped ?? "";
    this.title.value = "";
    this.typedTitle = false;
    this.clipped.hidden = true;
    this.submit.disabled = false;
    this.dialog.showModal();
    if (dropped === null) {
      const clipped = await this.adding.clipboard();
      if (clipped !== null && this.link.value === "") {
        this.link.value = clipped;
        this.clipped.hidden = false;
      }
    }
    this.link.select();
    await this.readTitle();
  }

  private async readTitle(): Promise<void> {
    const link = this.link.value.trim();
    if (link === "" || this.typedTitle) {
      return;
    }
    this.asked = link;
    this.title.placeholder = "Reading the page…";
    try {
      const title = await this.adding.title(link);
      if (this.asked === link && !this.typedTitle) {
        this.title.value = title ?? "";
      }
    } catch (error: unknown) {
      // A page that cannot be read leaves the title to the person, and the link stands in for it.
      this.title.placeholder = `The page could not be read: ${String(error)}`;
      return;
    }
    this.title.placeholder = "Read from the page";
  }

  private async send(): Promise<void> {
    this.submit.disabled = true;
    const added = await this.adding.add(this.link.value, this.title.value, this.collection.value);
    this.submit.disabled = false;
    if (added) {
      this.dialog.close();
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
  private id = "";

  /** The sheet that saves every field with `save` and moves the shortcut to the trash with `trashed`. */
  constructor(save: (fields: Fields) => void, trashed: (id: string) => void, pick: PickCollections) {
    this.collection = new Picker(find("edit-collection-search", HTMLInputElement), find("edit-collection", HTMLSelectElement), pick);
    closable(this.dialog);
    this.form.addEventListener("submit", (event) => {
      event.preventDefault();
      save({
        id: this.id,
        title: this.title.value,
        alias: this.alias.value.trim(),
        note: this.note.value,
        collection: this.collection.value,
      });
      this.dialog.close();
    });
    this.remove.addEventListener("click", () => {
      this.dialog.close();
      trashed(this.id);
    });
  }

  /** Opens the sheet over a shortcut. */
  open(item: Item): void {
    this.id = item.id;
    this.title.value = item.title;
    this.link.textContent = item.link;
    this.alias.value = item.alias;
    const home = item.collectionId === null || item.collection === null ? null : { id: item.collectionId, path: item.collection };
    void this.collection.open(home);
    this.note.value = item.note;
    this.dialog.showModal();
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
}

/** The sheet that asks for a name or a confirmation. */
export class AskSheet {
  private readonly dialog = find("ask-sheet", HTMLDialogElement);
  private readonly form = find("ask-form", HTMLFormElement);
  private readonly title = find("ask-title", HTMLElement);
  private readonly text = find("ask-text", HTMLElement);
  private readonly input = find("ask-input", HTMLInputElement);
  private readonly submit = find("ask-submit", HTMLButtonElement);
  private settle: ((answer: string | null) => void) | undefined;

  constructor() {
    closable(this.dialog);
    this.form.addEventListener("submit", (event) => {
      event.preventDefault();
      this.finish(this.input.value);
    });
    this.dialog.addEventListener("close", () => this.finish(null));
  }

  private finish(answer: string | null): void {
    const settle = this.settle;
    this.settle = undefined;
    this.input.value = "";
    if (this.dialog.open) {
      this.dialog.close();
    }
    settle?.(answer);
  }

  /** Asks, and answers with the text, an empty text for a confirmation, or none when the person cancels. */
  ask(question: Question): Promise<string | null> {
    this.finish(null);
    this.title.textContent = question.title;
    this.text.textContent = question.text;
    this.text.hidden = question.text === "";
    this.submit.textContent = question.action;
    this.submit.className = question.answer === null ? "danger" : "primary";
    this.input.hidden = question.answer === null;
    this.input.value = question.answer ?? "";
    this.input.type = question.secret === true ? "password" : "text";
    return new Promise((resolve) => {
      this.settle = resolve;
      this.dialog.showModal();
      if (question.answer === null) {
        this.submit.focus();
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
    closable(this.dialog);
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
    this.dialog.addEventListener("close", () => this.finish(null));
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
    this.preview.replaceChildren(shown === "" || /^[\x20-\x7e]*$/u.test(shown) ? icon("folder") : shown);
  }

  private async search(query: string): Promise<void> {
    this.asked += 1;
    const asked = this.asked;
    const groups = await this.pick(query);
    if (asked !== this.asked) {
      return;
    }
    this.first = query === "" ? "" : (groups[0]?.choices[0]?.emoji ?? "");
    this.show();
    this.grid.replaceChildren(
      ...groups.flatMap((group) => [
        el("h3", "mark-heading", group.label),
        ...(group.choices.length === 0 ? [el("p", "hint", "No emoji has that name.")] : []),
        ...group.choices.map((choice) => {
          const pick = el("button", "mark-choice", choice.emoji);
          pick.type = "button";
          pick.title = `:${choice.name}:`;
          pick.setAttribute("aria-label", choice.name.replaceAll("_", " "));
          pick.addEventListener("click", () => {
            this.chosen = choice.emoji;
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
    this.title.textContent = `Mark of ${name}`;
    this.chosen = mark;
    this.first = "";
    this.input.value = "";
    this.clear.disabled = mark === "";
    this.show();
    void this.search("");
    return new Promise((resolve) => {
      this.settle = resolve;
      this.dialog.showModal();
      this.input.focus();
    });
  }
}

/** A choice of a menu, which a check marks when it stands for something on or off, or a line between choices. */
export type MenuChoice = { readonly label: string; readonly danger?: boolean; readonly checked?: boolean; readonly act: () => void } | "line";

/** The menu a right click opens beside the pointer. */
export class Menu {
  private readonly menu = find("menu", HTMLElement);

  constructor() {
    document.addEventListener("click", () => this.close());
    window.addEventListener("blur", () => this.close());
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        this.close();
      }
    });
  }

  get open(): boolean {
    return !this.menu.hidden;
  }

  close(): void {
    this.menu.hidden = true;
  }

  show(event: MouseEvent, choices: readonly MenuChoice[]): void {
    event.preventDefault();
    this.menu.replaceChildren(
      ...choices.map((choice) => {
        if (choice === "line") {
          return el("hr");
        }
        const item = el("button", choice.danger === true ? "danger-item" : "", choice.label);
        item.type = "button";
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
    const { width, height } = this.menu.getBoundingClientRect();
    this.menu.style.left = `${Math.min(event.clientX, window.innerWidth - width)}px`;
    this.menu.style.top = `${Math.min(event.clientY, window.innerHeight - height)}px`;
  }
}
