// Wears the theme the library chooses, and draws the settings pane that chooses it.

import { button, el, icon } from "./dom.js";
import type { ThemeOption, Worn } from "./types.js";

/** The media query of the system's dark appearance. */
const DARK = window.matchMedia("(prefers-color-scheme: dark)");

let worn: Worn | null = null;

/** The theme to wear now: the dark one in a dark appearance when the window follows the system, the light one otherwise. */
function current(theme: Worn): ThemeOption {
  return theme.follow && DARK.matches ? theme.dark : theme.light;
}

function apply(): void {
  if (worn === null) {
    return;
  }
  const theme = current(worn);
  const root = document.documentElement;
  for (const [property, color] of Object.entries(theme.colors)) {
    root.style.setProperty(property, color);
  }
  root.style.setProperty("color-scheme", theme.kind);
}

/** Wears the themes a view names. */
export function wear(theme: Worn | null): void {
  worn = theme;
  apply();
}

DARK.addEventListener("change", apply);

/** What the theme pane asks of the library. */
export interface Choosing {
  choose(follow: boolean, light: string, dark: string): void;
  import(text: string): void;
  remove(id: string): void;
}

/** A small picture of the window in a theme's colors. */
function preview(theme: ThemeOption): HTMLElement {
  const frame = el("div", "theme-preview");
  for (const [property, color] of Object.entries(theme.colors)) {
    frame.style.setProperty(property, color);
  }
  const side = el("div", "mini-side");
  side.append(el("i"), el("i"), el("i"));
  const page = el("div", "mini-page");
  const type = el("div", "mini-type");
  type.append(el("span", "mini-scheme", "https://"), el("span", "mini-name", "whatwg"), el("span", "mini-suffix", ".org"));
  page.append(type, el("i", "mini-line"), el("i", "mini-button"));
  frame.append(side, page);
  return frame;
}

function picker(label: string, themes: readonly ThemeOption[], chosen: string, choose: (id: string) => void): HTMLElement {
  const field = el("label", "theme-picker");
  const select = el("select");
  for (const theme of themes) {
    const option = el("option", "", theme.name);
    option.value = theme.id;
    select.append(option);
  }
  select.value = chosen;
  select.addEventListener("change", () => choose(select.value));
  field.append(el("span", "", label), select);
  return field;
}

/** The tiles of one kind of theme, each choosing itself when clicked. */
function tiles(themes: readonly ThemeOption[], chosen: string, choose: (id: string) => void, remove: (id: string) => void): HTMLElement {
  const grid = el("div", "theme-grid");
  for (const theme of themes) {
    const tile = el("button", "theme-tile");
    tile.type = "button";
    tile.setAttribute("aria-pressed", String(theme.id === chosen));
    const caption = el("span", "theme-caption", theme.name);
    if (theme.id === chosen) {
      caption.append(icon("check"));
    }
    tile.append(preview(theme), caption);
    tile.addEventListener("click", () => choose(theme.id));
    if (theme.imported) {
      tile.addEventListener("contextmenu", (event) => {
        event.preventDefault();
        remove(theme.id);
      });
      tile.title = "Right-click to remove this imported theme";
    }
    grid.append(tile);
  }
  return grid;
}

/** The settings pane of the themes. */
export function themePane(pane: HTMLElement, themes: readonly ThemeOption[], theme: Worn, on: Choosing): void {
  const light = themes.filter((option) => option.kind === "light");
  const dark = themes.filter((option) => option.kind === "dark");
  const chooseLight = (id: string): void => on.choose(theme.follow, id, theme.dark.id);
  // Without following the system, the window wears one theme, whichever kind a person picks.
  const chooseDark = theme.follow ? (id: string): void => on.choose(theme.follow, theme.light.id, id) : chooseLight;
  const follow = el("label", "setting toggle");
  const toggle = el("input");
  toggle.type = "checkbox";
  toggle.checked = theme.follow;
  toggle.addEventListener("change", () => on.choose(toggle.checked, theme.light.id, theme.dark.id));
  follow.append(el("span", "", "Match the system's light and dark appearance"), toggle);
  const pickers = el("div", "pickers");
  pickers.append(picker(theme.follow ? "In light" : "Theme", themes, theme.light.id, chooseLight));
  if (theme.follow) {
    pickers.append(picker("In dark", themes, theme.dark.id, chooseDark));
  }
  const group = el("div", "group");
  group.append(follow, pickers);
  const file = el("input");
  file.type = "file";
  file.accept = ".json,application/json";
  file.hidden = true;
  file.addEventListener("change", () => {
    const chosen = file.files?.[0];
    file.value = "";
    if (chosen !== undefined) {
      void chosen.text().then((text) => on.import(text));
    }
  });
  const load = button("Import theme file…", "secondary");
  load.addEventListener("click", () => file.click());
  const footer = el("div", "theme-footer");
  footer.append(load, file, el("span", "hint", "VS Code color themes (.json) work as they are."));
  pane.replaceChildren(
    group,
    el("h3", "", "Light"),
    tiles(light, theme.light.id, chooseLight, on.remove),
    el("h3", "", "Dark"),
    tiles(dark, theme.follow ? theme.dark.id : theme.light.id, chooseDark, on.remove),
    footer,
  );
}
