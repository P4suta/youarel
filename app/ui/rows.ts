// The geometry of a long listing that the page draws a window of: the lines its rows and headings take, the lines a scroll shows, and the rows the page holds so far.
// A listing may hold millions of rows, so the page holds the rows around the place in sight and asks the library for others as a person scrolls.

/** A run of rows under a heading, or under none, which starts a line of its own. */
export interface Section {
  readonly start: number;
  readonly count: number;
  readonly label: string | null;
}

/** The sections of a listing of `total` rows: each mark in turn, and the rows before, between, and after the marks under no heading. */
export function sections(total: number, marks: readonly Section[]): Section[] {
  const found: Section[] = [];
  let place = 0;
  for (const mark of marks) {
    if (mark.start > place) {
      found.push({ start: place, count: mark.start - place, label: null });
    }
    if (mark.count > 0) {
      found.push(mark);
    }
    place = Math.max(place, mark.start + mark.count);
  }
  if (place < total) {
    found.push({ start: place, count: total - place, label: null });
  }
  return found;
}

/** How a listing draws: how many rows share a line, the height a line of rows takes with the gap after it, and the height of a heading. */
export interface Metrics {
  readonly across: number;
  readonly pitch: number;
  readonly heading: number;
}

/** One line the page draws: a heading at its top, or a line of rows from a place on. */
export type Line =
  | { readonly kind: "heading"; readonly section: Section; readonly top: number }
  | { readonly kind: "rows"; readonly first: number; readonly count: number; readonly top: number };

/** The halves a search of a sorted list splits its range into. */
const HALVES = 2;

/** Where every line of a listing lies, read from its sections alone, so the page needs none of the rows to place them. */
export class Geometry {
  private readonly tops: readonly number[];
  readonly height: number;

  constructor(
    readonly parts: readonly Section[],
    readonly metrics: Metrics,
  ) {
    const tops: number[] = [];
    let top = 0;
    for (const part of parts) {
      tops.push(top);
      top += this.headingOf(part) + this.linesOf(part) * metrics.pitch;
    }
    this.tops = tops;
    this.height = top;
  }

  private headingOf(part: Section): number {
    return part.label === null ? 0 : this.metrics.heading;
  }

  private linesOf(part: Section): number {
    return Math.ceil(part.count / this.metrics.across);
  }

  /** The index of the last section that starts at or before a place. */
  private holding(place: number): number {
    let low = 0;
    let high = this.parts.length - 1;
    while (low < high) {
      const middle = Math.ceil((low + high) / HALVES);
      if ((this.parts[middle]?.start ?? 0) <= place) {
        low = middle;
      } else {
        high = middle - 1;
      }
    }
    return low;
  }

  /** The index of the last section that starts at or above a height. */
  private reaching(height: number): number {
    let low = 0;
    let high = this.tops.length - 1;
    while (low < high) {
      const middle = Math.ceil((low + high) / HALVES);
      if ((this.tops[middle] ?? 0) <= height) {
        low = middle;
      } else {
        high = middle - 1;
      }
    }
    return low;
  }

  /** The top of the line that holds a place. */
  top(place: number): number {
    const index = this.holding(place);
    const part = this.parts[index];
    const top = this.tops[index];
    if (part === undefined || top === undefined) {
      return 0;
    }
    const line = Math.floor(Math.max(0, place - part.start) / this.metrics.across);
    return top + this.headingOf(part) + line * this.metrics.pitch;
  }

  /** The lines that a band from one height to another touches, in their order. */
  lines(from: number, to: number): Line[] {
    const lines: Line[] = [];
    const { across, pitch } = this.metrics;
    for (let index = this.reaching(from); index < this.parts.length; index += 1) {
      const part = this.parts[index];
      const top = this.tops[index];
      if (part === undefined || top === undefined || top >= to) {
        break;
      }
      const heading = this.headingOf(part);
      if (heading > 0 && top + heading > from) {
        lines.push({ kind: "heading", section: part, top });
      }
      const body = top + heading;
      const firstLine = Math.max(0, Math.floor((from - body) / pitch));
      const endLine = Math.min(this.linesOf(part), Math.ceil((to - body) / pitch));
      for (let line = firstLine; line < endLine; line += 1) {
        const first = part.start + line * across;
        lines.push({ kind: "rows", first, count: Math.min(across, part.start + part.count - first), top: body + line * pitch });
      }
    }
    return lines;
  }

  /** The places of the rows a band shows, from the first to past the last, or an empty span at the start for a band of no rows. */
  span(from: number, to: number): { readonly first: number; readonly end: number } {
    let first = Number.POSITIVE_INFINITY;
    let end = 0;
    for (const line of this.lines(from, to)) {
      if (line.kind === "rows") {
        first = Math.min(first, line.first);
        end = Math.max(end, line.first + line.count);
      }
    }
    return Number.isFinite(first) ? { first, end } : { first: 0, end: 0 };
  }
}

/** The tallest frame a listing takes in the page, within the 33,554,431 pixels a browser lays out, past which a scroll reaches the lines in proportion. */
const MOST_FRAME = 30_000_000;

/** The frame of a listing in the page, and how a scroll of it maps onto the heights of the listing's lines. */
export class Frame {
  /** The frame's height in the page. */
  readonly height: number;

  constructor(readonly lines: number) {
    this.height = Math.min(lines, MOST_FRAME);
  }

  /** How many heights of the lines one height of the frame stands for while a band of `client` shows. */
  private ratio(client: number): number {
    const room = this.height - client;
    return this.lines > this.height && room > 0 ? (this.lines - client) / room : 1;
  }

  /** The height of the lines at the top of the band that a scroll of the frame shows. */
  top(scroll: number, client: number): number {
    return scroll * this.ratio(client);
  }

  /** The scroll of the frame that shows a height of the lines at the top of its band. */
  scroll(top: number, client: number): number {
    return top / this.ratio(client);
  }

  /** Where a line at a height of the lines sits in the frame while a scroll shows its band. */
  place(line: number, scroll: number, client: number): number {
    return scroll + line - this.top(scroll, client);
  }
}

/** The most rows the page holds of one listing before it lets the ones far from sight go. */
const MOST_HELD = 5000;

/** The rows the page holds of one listing, by their places, and what tells that listing apart. */
export class Rows<T> {
  private held = new Map<number, T>();
  key = "";
  total = 0;
  /** How many times the rows held changed, which tells a drawing whether to draw again. */
  version = 0;

  /** Holds the rows of a view of a listing in place of every row held before, since a view may change any of them. */
  reset(key: string, total: number, first: number, rows: readonly T[]): void {
    this.held = new Map();
    this.key = key;
    this.take(total, first, rows);
  }

  /** Holds the rows of a page of the listing held, and lets go of the rows far from it once the page holds many. */
  take(total: number, first: number, rows: readonly T[]): void {
    this.total = total;
    this.version += 1;
    if (this.held.size + rows.length > MOST_HELD) {
      this.held = new Map();
    }
    rows.forEach((row, offset) => {
      this.held.set(first + offset, row);
    });
  }

  get(place: number): T | undefined {
    return this.held.get(place);
  }

  /** Whether the page holds every row of a span. */
  holds(first: number, end: number): boolean {
    for (let place = first; place < end; place += 1) {
      if (!this.held.has(place)) {
        return false;
      }
    }
    return true;
  }
}
