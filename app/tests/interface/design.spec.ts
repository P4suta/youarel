import { expect, test } from "@playwright/test";
import axe from "axe-core";
import { computedFindings, layoutOverflow, paletteFindings } from "./design.js";
import { BOOKMARK, library, newBookmark, rowMenu } from "./fixture.js";

for (const scheme of ["light", "dark"] as const) {
	test(`the ${scheme} library preserves contrast, targets, and named dialogs`, async ({
		page,
	}, info): Promise<void> => {
		await page.emulateMedia({ colorScheme: scheme });
		await library(page);
		expect(await paletteFindings(page)).toEqual([]);
		expect(await computedFindings(page)).toEqual([]);
		const targets = await page
			.locator("#content .row-open")
			.evaluateAll((elements): readonly string[] =>
				elements
					.filter(
						(element) =>
							element.getClientRects().length > 0 &&
							(element.getBoundingClientRect().width < 48 ||
								element.getBoundingClientRect().height < 48),
					)
					.map(
						(element) =>
							element.getAttribute("aria-label") ?? element.className,
					),
			);
		expect(targets).toEqual([]);
		await page.screenshot({ path: info.outputPath("library.png") });
		await page
			.getByRole("button", { name: `Open ${BOOKMARK.title}`, exact: true })
			.click({ modifiers: ["ControlOrMeta"] });
		await page
			.getByRole("button", {
				name: `Open ${BOOKMARK.title}`,
				exact: true,
			})
			.focus();
		expect(await computedFindings(page)).toEqual([]);
		await page.addScriptTag({ content: axe.source });
		await newBookmark(page);
		const violations = await page.evaluate(
			async (): Promise<readonly string[]> =>
				(await window.axe.run(document)).violations.map((entry) => entry.id),
		);
		expect(violations).toEqual([]);
		await page
			.getByRole("dialog", { name: "Add a link" })
			.getByRole("button", { name: "Cancel", exact: true })
			.click();
		await rowMenu(page, BOOKMARK.title);
		await page.getByRole("menuitem", { name: "Edit…", exact: true }).click();
		const editViolations = await page.evaluate(
			async (): Promise<readonly string[]> =>
				(await window.axe.run(document)).violations.map((entry) => entry.id),
		);
		expect(editViolations).toEqual([]);
		expect(await computedFindings(page)).toEqual([]);
		await page.screenshot({ path: info.outputPath("edit.png") });
	});
}

for (const size of [
	{ width: 320, height: 680 },
	{ width: 760, height: 480 },
	{ width: 1180, height: 760 },
	{ width: 680, height: 320 },
]) {
	test(`the ${size.width} by ${size.height} library survives text enlargement and spacing`, async ({
		page,
	}, info): Promise<void> => {
		await page.setViewportSize(size);
		await library(page);
		await page.addStyleTag({
			content:
				"* { line-height: 1.5 !important; letter-spacing: .12em !important; word-spacing: .16em !important; } p { margin-bottom: 2em !important; }",
		});
		for (let step = 0; step < 4; step += 1)
			await page.keyboard.press("Control+=");
		await expect(page.locator("body")).toHaveClass(/compact/u);
		expect(await layoutOverflow(page)).toEqual([]);
		const actions = page.getByRole("button", {
			name: `Open ${BOOKMARK.title}`,
			exact: true,
		});
		await actions.focus();
		await expect(actions).toBeInViewport();
		await newBookmark(page);
		const dialog = page.getByRole("dialog", { name: "Add a link" });
		await dialog.getByLabel("URL", { exact: true }).fill(BOOKMARK.link);
		expect(await layoutOverflow(page)).toEqual([]);
		const cancel = dialog.getByRole("button", { name: "Cancel", exact: true });
		await cancel.focus();
		await expect(cancel).toBeInViewport();
		await page.screenshot({ path: info.outputPath("enlarged-form.png") });
		await page.keyboard.press("Escape");
		await page.keyboard.press("Control+0");
		await expect(page.locator("html")).toHaveCSS("font-size", "16px");
	});
}

test("the palette check accepts black and white and refuses each prohibited effect", async ({
	page,
}): Promise<void> => {
	await page.setContent("<main>Palette fixture</main>");
	const valid = await page.addStyleTag({
		content:
			"main { color: #000; background: #fff; border: 1px solid currentColor; opacity: 1; box-shadow: none; filter: none; mix-blend-mode: normal; }",
	});
	expect(await paletteFindings(page)).toEqual([]);
	await valid.evaluate((element): void => {
		if (element instanceof Element) element.remove();
	});
	for (const declaration of [
		"color: red",
		"background: #eee",
		"background: rgb(0 0 0 / .5)",
		"opacity: .5",
		"background: linear-gradient(#000, #fff)",
		"box-shadow: 0 0 2px #000",
		"filter: blur(2px)",
		"backdrop-filter: blur(2px)",
		"mix-blend-mode: multiply",
	]) {
		const style = await page.addStyleTag({
			content: `main { ${declaration}; }`,
		});
		expect(await paletteFindings(page), declaration).not.toEqual([]);
		await style.evaluate((element): void => {
			if (element instanceof Element) element.remove();
		});
	}
});

test("forced colors keep focus and selection discernible", async ({
	page,
}): Promise<void> => {
	await page.emulateMedia({ forcedColors: "active" });
	await library(page);
	const open = page.getByRole("button", {
		name: `Open ${BOOKMARK.title}`,
		exact: true,
	});
	await open.focus();
	await expect(open).toHaveCSS("outline-style", "dashed");
	await expect(open).toHaveCSS("outline-width", "2px");
	await open.click();
	await expect(page.locator(".bookmark-row")).toHaveCSS(
		"outline-style",
		"solid",
	);
	await expect(page.locator(".bookmark-row")).toHaveCSS("outline-width", "2px");
	expect(await layoutOverflow(page)).toEqual([]);
});
