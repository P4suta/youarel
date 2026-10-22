// What crosses between the page and the library: the view the library draws, and the intents the page sends.
// The shapes mirror `app/src/view.rs` and `app/src/intent.rs`, which name every field the same way.

export type Layout = "cards" | "list" | "columns";
export type Sort = "newest" | "oldest" | "title" | "link" | "opened" | "manual";
/** A column of the list that a person may hide. */
export type ColumnName = "link" | "collection" | "added";
export type Grouping = "none" | "site" | "collection" | "added";
export type Key = "up" | "down" | "left" | "right" | "first" | "last";
export type Extend = "replace" | "toggle" | "range";

export type ScopeName =
  | { readonly kind: "all" }
  | { readonly kind: "unsorted" }
  | { readonly kind: "recent" }
  | { readonly kind: "collection"; readonly id: string };

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

/** A folder of the sidebar's tree: its count, how deep it sits, whether it holds folders, and whether it shows them. */
export interface SidebarFolder extends CollectionCount {
  readonly depth: number;
  readonly opens: boolean;
  readonly open: boolean;
}

export interface Sidebar {
  readonly scope: ScopeName;
  readonly all: number;
  readonly unsorted: number;
  readonly recent: number;
  /** The collections from the top, each open one followed by the folders inside it. */
  readonly collections: readonly SidebarFolder[];
}

export interface ColumnEntry {
  readonly kind: "folder" | "shortcut";
  readonly id: string;
  readonly name: string;
  readonly selected: boolean;
  readonly opens: boolean;
  readonly icon: string | null;
  /** The registrable name of a shortcut's site, which an entry without an icon shows as a tile, or empty for a folder. */
  readonly site: string;
  /** What a folder shows in place of its glyph, or empty for the glyph and for a shortcut. */
  readonly mark: string;
  /** The heading this entry and the shortcuts after it fall under, on the first of each. */
  readonly heading: string | null;
}

export interface Choice {
  readonly id: string;
  readonly path: string;
}

export interface Notice {
  readonly text: string;
  readonly error: boolean;
}

export interface ThemeOption {
  readonly id: string;
  readonly name: string;
  readonly kind: "light" | "dark";
  readonly imported: boolean;
  /** Each custom property of the style sheet with its color. */
  readonly colors: Readonly<Record<string, string>>;
}

export interface Worn {
  readonly follow: boolean;
  readonly light: ThemeOption;
  readonly dark: ThemeOption;
}

export interface View {
  readonly sidebar: Sidebar;
  readonly title: string;
  readonly count: number;
  readonly layout: Layout;
  readonly sort: Sort;
  readonly grouping: Grouping;
  readonly search: string;
  /** The folders the cards and the list show before the shortcuts while they stand in a folder. */
  readonly folders: readonly CollectionCount[];
  /** The columns the list draws after the name, in their order. */
  readonly listColumns: readonly ColumnName[];
  /** The rows of the list's outline, in their order, or none when the list shows the folders and the items as they come. */
  readonly outline: readonly OutlineRow[];
  /** The columns of the list a person hid. */
  readonly hiddenColumns: readonly ColumnName[];
  readonly items: readonly Item[];
  /** The headings the items fall under in turn, or none. */
  readonly groups: readonly Group[];
  readonly columns: readonly (readonly ColumnEntry[])[];
  readonly preview: Item | null;
  readonly focus: string | null;
  readonly selected: readonly string[];
  readonly notice: Notice | null;
  readonly collections: readonly Choice[];
  /** The folder a new folder or a new link goes into, or null for the top. */
  readonly here: string | null;
  /** The folders the columns open, from the left, or, in the cards and the list, the folders from the top down to the one they stand in. */
  readonly trail: readonly Step[];
  readonly theme: Worn | null;
  /** The browsers that keep bookmarks on this kind of host. */
  readonly browsers: readonly Browser[];
  readonly sync: SyncView;
}

export interface Group {
  readonly label: string;
  /** How many of the items that follow the heading fall under it. */
  readonly count: number;
}

/** One row of the list's outline: a folder or a shortcut, how deep it sits, and whether a folder shows its rows. */
export interface OutlineRow {
  readonly kind: "folder" | "shortcut";
  readonly id: string;
  readonly depth: number;
  readonly open: boolean;
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

export interface SyncView {
  readonly enabled: boolean;
  readonly target: string;
  readonly state: "off" | "noAccount" | "waiting" | "syncing" | "synced" | "failed";
  readonly last: number | null;
  readonly error: string | null;
}

export type Browser = "safari" | "chrome" | "firefox";

export interface Fields {
  readonly id: string;
  readonly title: string;
  readonly alias: string;
  readonly note: string;
  readonly collection: string | null;
}

export type Intent =
  | { readonly kind: "show" }
  | { readonly kind: "scope"; readonly scope: ScopeName }
  | { readonly kind: "layout"; readonly layout: Layout }
  | { readonly kind: "sort"; readonly sort: Sort }
  | { readonly kind: "group"; readonly grouping: Grouping }
  | { readonly kind: "showColumn"; readonly column: ColumnName; readonly shown: boolean }
  | { readonly kind: "search"; readonly text: string }
  | { readonly kind: "select"; readonly id: string; readonly extend: Extend }
  | { readonly kind: "selectAll" }
  | { readonly kind: "disclose"; readonly id: string; readonly open: boolean }
  | { readonly kind: "discloseSidebar"; readonly id: string; readonly open: boolean }
  | { readonly kind: "markCollection"; readonly id: string; readonly mark: string }
  | { readonly kind: "step"; readonly key: Key; readonly across: number }
  | {
      readonly kind: "column";
      readonly depth: number;
      readonly entry: { readonly kind: "folder" | "shortcut"; readonly id: string };
    }
  | { readonly kind: "open"; readonly id: string | null }
  | { readonly kind: "edit"; readonly fields: Fields }
  | { readonly kind: "retitle"; readonly id: string; readonly title: string }
  | { readonly kind: "file"; readonly ids: readonly string[]; readonly collection: string | null }
  | { readonly kind: "remove"; readonly ids: readonly string[] }
  | { readonly kind: "newCollection"; readonly name: string; readonly parent: string | null }
  | { readonly kind: "renameCollection"; readonly id: string; readonly name: string }
  | { readonly kind: "removeCollection"; readonly id: string }
  | { readonly kind: "moveFolders"; readonly ids: readonly string[]; readonly collection: string | null }
  | { readonly kind: "import"; readonly bytes: readonly number[] }
  | { readonly kind: "theme"; readonly follow: boolean; readonly light: string; readonly dark: string }
  | { readonly kind: "importTheme"; readonly text: string }
  | { readonly kind: "removeTheme"; readonly id: string }
  | { readonly kind: "importBrowser"; readonly browser: Browser }
  | { readonly kind: "syncSettings"; readonly enabled: boolean; readonly target: string }
  | { readonly kind: "syncNow" };

interface Tauri {
  readonly core: {
    invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  };
  readonly event: {
    listen(event: string, handler: (raised: { readonly payload: unknown }) => void): Promise<() => void>;
  };
}

declare global {
  interface Window {
    readonly __TAURI__: Tauri;
  }
}

/** Calls a command of the library. */
export function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  return window.__TAURI__.core.invoke<T>(command, args);
}

/** Calls `handler` each time the library raises `event`. */
export function listen(event: string, handler: () => void): Promise<() => void> {
  return window.__TAURI__.event.listen(event, () => handler());
}

/** Calls `handler` with the text each time the library raises `event` with one, and ignores a payload that is no text. */
export function listenText(event: string, handler: (text: string) => void): Promise<() => void> {
  return window.__TAURI__.event.listen(event, (raised) => {
    if (typeof raised.payload === "string") {
      handler(raised.payload);
    }
  });
}
