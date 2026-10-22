// The sheets: adding a link, editing a shortcut, asking for a name or a confirmation, and the menu of one shortcut.

import { el, find, icon } from "./dom.js";
import type { Choice, EmojiGroup, Fields, Item } from "./types.js";

/** How long the add sheet waits after the last keystroke in the link before it reads the page's title. */
const TITLE_DELAY_MS = 600;

/** Fills a collection picker with every collection and none, choosing `chosen`. */
function collections(select: HTMLSelectElement, choices: readonly Choice[], chosen: string | null): void {
  const none = el("option", "", "No collection");
  none.value = "";
  select.replaceChildren(
    none,
    ...choices.map((choice) => {
      const option = el("option", "", choice.path);
      option.value = choice.id;
      return option;
    }),
  );
  select.value = chosen ?? "";
}

function chosen(select: HTMLSelectElement): string | null {
  return select.value === "" ? null : select.value;
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
  private readonly collection = find("add-collection", HTMLSelectElement);
  private readonly clipped = find("add-from-clipboard", HTMLElement);
  private readonly submit = find("add-submit", HTMLButtonElement);
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
      window.clearTimeout(this.timer);
      this.timer = window.setTimeout(() => void this.readTitle(), TITLE_DELAY_MS);
    });
    this.form.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.send();
    });
  }

  /** Opens the sheet with a link dropped on the window, or the one on the clipboard. */
  async open(choices: readonly Choice[], collection: string | null, dropped: string | null): Promise<void> {
    collections(this.collection, choices, collection);
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
    const added = await this.adding.add(this.link.value, this.title.value, chosen(this.collection));
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
  private readonly collection = find("edit-collection", HTMLSelectElement);
  private readonly note = find("edit-note", HTMLTextAreaElement);
  private readonly remove = find("edit-remove", HTMLButtonElement);
  private id = "";

  constructor(save: (fields: Fields) => void, removed: (id: string) => void) {
    closable(this.dialog);
    this.form.addEventListener("submit", (event) => {
      event.preventDefault();
      save({
        id: this.id,
        title: this.title.value,
        alias: this.alias.value.trim(),
        note: this.note.value,
        collection: chosen(this.collection),
      });
      this.dialog.close();
    });
    this.remove.addEventListener("click", () => {
      this.dialog.close();
      removed(this.id);
    });
  }

  /** Opens the sheet over a shortcut. */
  open(item: Item, choices: readonly Choice[]): void {
    this.id = item.id;
    this.title.value = item.title;
    this.link.textContent = item.link;
    this.alias.value = item.alias;
    collections(this.collection, choices, item.collectionId);
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
