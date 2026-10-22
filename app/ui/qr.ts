export interface CodeImage {
  readonly rows: readonly (readonly boolean[])[];
}

const SMALLEST_SIDE = 21;
const LARGEST_SIDE = 177;
const SIDE_STEP = 4;
const QUIET = 4;
const BORDER = QUIET * 2;
const SCALE = 6;
const SVG = "http://www.w3.org/2000/svg";

export function codeImage(value: unknown): value is CodeImage {
  if (typeof value !== "object" || value === null || !("rows" in value) || !Array.isArray(value.rows)) { return false; }
  const side = value.rows.length;
  return side >= SMALLEST_SIDE && side <= LARGEST_SIDE && (side - SMALLEST_SIDE) % SIDE_STEP === 0
    && Array.from(value.rows).every((row: unknown) => Array.isArray(row) && row.length === side
      && Array.from(row).every((cell: unknown) => typeof cell === "boolean"));
}

export function drawCodeImage(target: HTMLElement, image: CodeImage | null): void {
  target.replaceChildren();
  target.hidden = image === null;
  if (image === null) { return; }
  const side = image.rows.length + BORDER;
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", `0 0 ${side} ${side}`);
  svg.setAttribute("width", String(side * SCALE));
  svg.setAttribute("height", String(side * SCALE));
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", "Invitation code QR");
  svg.setAttribute("shape-rendering", "crispEdges");
  const background = document.createElementNS(SVG, "rect");
  background.setAttribute("width", String(side));
  background.setAttribute("height", String(side));
  background.setAttribute("fill", "white");
  const cells: string[] = [];
  image.rows.forEach((row, y) => row.forEach((dark, x) => {
    if (dark) { cells.push(`M${x + QUIET},${y + QUIET}h1v1h-1z`); }
  }));
  const path = document.createElementNS(SVG, "path");
  path.setAttribute("d", cells.join(""));
  path.setAttribute("fill", "black");
  svg.append(background, path);
  target.append(svg);
}
