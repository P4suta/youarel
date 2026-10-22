// A link set as type: on a card the registrable name stands out, and in a row the whole host does; the scheme and the path recede.

import { el } from "./dom.js";
import type { UrlView } from "./types.js";

/** The rest of a link worth showing: nothing for a bare root. */
function rest(url: UrlView): string {
  return url.rest === "/" ? "" : url.rest;
}

/** The thumbnail of a card: the scheme in a corner, the subdomains above, the name and its suffix large, the path below. */
export function thumbnail(url: UrlView): HTMLElement {
  const block = el("div", "type");
  block.append(el("span", "scheme", url.scheme));
  const host = el("div", "host");
  // Every line keeps its place even when empty, so the name sits at one height on every card.
  host.append(el("span", "sub", url.sub));
  const line = el("div", "line");
  line.append(el("span", "name", url.name));
  if (url.suffix !== "") {
    line.append(el("span", "suffix", url.suffix));
  }
  if (url.port !== "") {
    line.append(el("span", "port", url.port));
  }
  host.append(line);
  block.append(host, el("span", "rest", rest(url)));
  return block;
}

/** A link on one line, as a list row or a preview shows it: its whole host, which tells `mail.google.com` from `google.com`, and its path receding. */
export function inline(url: UrlView): HTMLElement {
  const line = el("span", "url");
  line.append(el("span", "host", `${url.sub}${url.name}${url.suffix}${url.port}`));
  const path = rest(url);
  if (path !== "") {
    line.append(el("span", "rest", path));
  }
  return line;
}
