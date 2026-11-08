// Small builders of the page's elements, which set every text as text and never as markup.

const SVG = "http://www.w3.org/2000/svg";

/** An element of `tag` with a class and, if given, its text. */
export function el<K extends keyof HTMLElementTagNameMap>(
	tag: K,
	className = "",
	text?: string,
): HTMLElementTagNameMap[K] {
	const element = document.createElement(tag);
	if (className !== "") {
		element.className = className;
	}
	if (text !== undefined) {
		element.textContent = text;
	}
	return element;
}

/** An icon of the page's sprite, by its symbol's name. */
export function icon(name: string, className = "glyph"): SVGSVGElement {
	const svg = document.createElementNS(SVG, "svg");
	svg.setAttribute("class", className);
	svg.setAttribute("aria-hidden", "true");
	const use = document.createElementNS(SVG, "use");
	use.setAttribute("href", `#icon-${name}`);
	svg.append(use);
	return svg;
}

/** A button with an optional icon and a label. */
export function button(
	label: string,
	className = "",
	glyph?: string,
): HTMLButtonElement {
	const made = el("button", className);
	made.type = "button";
	if (glyph !== undefined) {
		made.append(icon(glyph));
	}
	if (label !== "") {
		made.append(el("span", "label", label));
	}
	return made;
}

/** The element the page holds under `id`, of the kind it must be. */
export function find<T extends Element>(
	id: string,
	kind: abstract new () => T,
): T {
	const found = document.getElementById(id);
	if (!(found instanceof kind)) {
		throw new Error(`the page lacks #${id}`);
	}
	return found;
}

export function containFocus(dialog: HTMLDialogElement): void {
	dialog.addEventListener("keydown", (event) => {
		if (event.key !== "Tab") {
			return;
		}
		const controls = [
			...dialog.querySelectorAll<HTMLElement>(
				"button, input, select, textarea, a[href], [tabindex]",
			),
		].filter(
			(element) =>
				element.tabIndex >= 0 &&
				!element.matches(":disabled") &&
				element.getClientRects().length > 0,
		);
		const first = controls[0];
		const last = controls.at(-1);
		if (
			event.shiftKey &&
			(document.activeElement === first ||
				!controls.some((element) => element === document.activeElement))
		) {
			event.preventDefault();
			last?.focus();
		} else if (!event.shiftKey && document.activeElement === last) {
			event.preventDefault();
			first?.focus();
		}
	});
}
