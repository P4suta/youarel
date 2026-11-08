import type { Page } from "@playwright/test";

export async function paletteFindings(page: Page): Promise<readonly string[]> {
	return page.evaluate((): string[] => {
		const findings: string[] = [];
		const color = (value: string, forced: boolean): boolean => {
			const normalized = value.toLowerCase().replaceAll(/\s+/gu, "");
			return (
				[
					"#000",
					"#000000",
					"#fff",
					"#ffffff",
					"black",
					"white",
					"rgb(0,0,0)",
					"rgb(255,255,255)",
					"currentcolor",
					"inherit",
					"var(--text)",
					"var(--background)",
					"none",
				].includes(normalized) ||
				(forced && normalized === "highlight")
			);
		};
		const inspect = (
			style: CSSStyleDeclaration,
			owner: string,
			forced: boolean,
		): void => {
			const properties = new Set([
				...style,
				"background-color",
				"background-image",
				"border-top-color",
				"border-right-color",
				"border-bottom-color",
				"border-left-color",
				"outline-color",
				"box-shadow",
				"text-shadow",
				"filter",
				"backdrop-filter",
				"mix-blend-mode",
				"background-blend-mode",
				"opacity",
			]);
			for (const property of properties) {
				const value = style.getPropertyValue(property);
				if (value === "") continue;
				const isColor =
					property === "color" ||
					property.endsWith("-color") ||
					["fill", "stroke", "--text", "--background"].includes(property);
				const badColor = isColor && !color(value, forced);
				const badEffect =
					(property === "opacity" && Number(value) !== 1) ||
					(property.includes("shadow") && value !== "none") ||
					(property.includes("filter") && value !== "none") ||
					(property.includes("blend-mode") && value !== "normal") ||
					value.includes("gradient(");
				if (badColor || badEffect)
					findings.push(`${owner}: ${property}: ${value}`);
			}
		};
		const rules = (entries: CSSRuleList, forced = false): void => {
			for (const rule of entries) {
				if (rule instanceof CSSStyleRule)
					inspect(rule.style, rule.selectorText, forced);
				if (rule instanceof CSSGroupingRule)
					rules(
						rule.cssRules,
						forced ||
							(rule instanceof CSSMediaRule &&
								rule.conditionText.includes("forced-colors: active")),
					);
			}
		};
		for (const sheet of document.styleSheets) rules(sheet.cssRules);
		for (const element of document.querySelectorAll<HTMLElement>("[style]"))
			inspect(element.style, element.tagName, false);
		return findings;
	});
}

export async function layoutOverflow(page: Page): Promise<readonly string[]> {
	return page.evaluate((): string[] =>
		[
			document.documentElement,
			...document.querySelectorAll<HTMLElement>(
				"#location, #name-heading, #content, dialog[open], dialog[open] form",
			),
		]
			.filter((element) => element.scrollWidth > element.clientWidth + 1)
			.map((element) => element.id || element.tagName),
	);
}

export async function computedFindings(page: Page): Promise<readonly string[]> {
	return page.evaluate((): string[] => {
		const findings: string[] = [];
		const colors = new Set([
			"rgb(0, 0, 0)",
			"rgb(255, 255, 255)",
			"rgba(0, 0, 0, 0)",
		]);
		for (const element of document.querySelectorAll<HTMLElement>(
			"body, body *",
		)) {
			if (
				element.getClientRects().length === 0 ||
				getComputedStyle(element).visibility === "hidden"
			)
				continue;
			for (const pseudo of [null, "::before", "::after"]) {
				const style = getComputedStyle(element, pseudo);
				if (pseudo !== null && ["none", "normal"].includes(style.content))
					continue;
				for (const property of [
					"color",
					"background-color",
					"border-top-color",
					"outline-color",
					"text-decoration-color",
				]) {
					const value = style.getPropertyValue(property);
					if (!colors.has(value))
						findings.push(
							`${element.tagName}${pseudo ?? ""}: ${property}: ${value}`,
						);
				}
				if (
					style.opacity !== "1" ||
					style.boxShadow !== "none" ||
					style.textShadow !== "none" ||
					style.filter !== "none" ||
					style.backdropFilter !== "none" ||
					style.mixBlendMode !== "normal" ||
					style.backgroundImage !== "none"
				)
					findings.push(`${element.tagName}${pseudo ?? ""}: visual effect`);
			}
		}
		return findings;
	});
}
