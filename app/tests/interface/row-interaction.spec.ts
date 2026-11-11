import { expect, type Locator, test } from "@playwright/test";
import type { Intent } from "../../ui/types.js";
import { BOOKMARK, type Call, library } from "./fixture.js";

function acts(calls: readonly Call[], kind: Intent["kind"]): readonly Call[] {
	return calls.filter(
		(call) =>
			call.command === "act" &&
			typeof call.args.intent === "object" &&
			call.args.intent !== null &&
			"kind" in call.args.intent &&
			call.args.intent.kind === kind,
	);
}

async function colors(row: Locator): Promise<readonly string[]> {
	return row.evaluate((element): readonly string[] => {
		const style = getComputedStyle(element);
		return [style.backgroundColor, style.color];
	});
}

for (const entry of [
	{ name: BOOKMARK.title, selector: ".bookmark-row" },
	{ name: "Research", selector: ".folder-row" },
	{ name: "Trash", selector: ".system-folder" },
]) {
	for (const part of ["icon", "name", "background"] as const) {
		test(`a single click on the ${entry.name} ${part} selects without opening`, async ({
			page,
		}): Promise<void> => {
			const calls = await library(page);
			const row = page.locator(entry.selector);
			if (part === "background") {
				const box = await row.boundingBox();
				if (box === null) throw new Error("The entry must be visible.");
				await row.click({ position: { x: box.width - 2, y: box.height / 2 } });
			} else {
				await row.locator(part === "icon" ? ".glyph" : ".item-name").click();
			}
			await expect(row).toHaveAttribute("data-selected", "true");
			await expect(row.locator(".row-open")).toBeFocused();
			expect(acts(calls, "open")).toHaveLength(0);
			expect(acts(calls, "scope")).toHaveLength(0);
		});
	}

	test(`${entry.name} distinguishes hover, selection, and keyboard focus`, async ({
		page,
	}): Promise<void> => {
		await library(page);
		const row = page.locator(entry.selector);
		await page.mouse.move(1170, 740);
		const resting = await colors(row);
		await row.hover();
		expect(await colors(row)).not.toEqual(resting);
		await expect(row).not.toHaveAttribute("data-selected", "true");
		await row.click();
		await page.mouse.move(1170, 740);
		await expect(row).toHaveAttribute("data-selected", "true");
		expect(await colors(row)).not.toEqual(resting);
		await expect(row).toHaveCSS("outline-style", "solid");
		await expect(row).toHaveCSS("outline-width", "2px");
		await page.locator("#content").focus();
		await page.keyboard.press("Tab");
		await row.locator(".row-open").focus();
		await expect(row.locator(".row-open")).toHaveCSS("outline-style", "dashed");
	});
}

for (const selector of [".bookmark-row", ".folder-row"]) {
	test(`${selector} keeps focus throughout selection updates`, async ({
		page,
	}): Promise<void> => {
		await library(page);
		const row = page.locator(selector);
		const target = row.locator(".row-open");
		await target.focus();
		await target.evaluate((element): void => {
			element.addEventListener("blur", (): void => {
				element.setAttribute("data-focus-lost", "true");
			});
		});
		await target.click();
		await expect(row).toHaveAttribute("data-selected", "true");
		await expect(target).toBeFocused();
		await expect(target).not.toHaveAttribute("data-focus-lost", "true");
	});
}

test("repeated plain clicks retain the selection and modifiers never open a link", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	const bookmark = page.locator(".bookmark-row .row-open");
	await bookmark.click();
	await expect(bookmark).toHaveAccessibleDescription("Bookmark Selected");
	await bookmark.click();
	await expect(bookmark).toHaveAccessibleDescription("Bookmark Selected");
	await bookmark.dblclick({ modifiers: ["ControlOrMeta"], delay: 100 });
	expect(acts(calls, "open")).toHaveLength(0);
});

test("bookmark keyboard activation opens once without timed key presses", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	await page.locator(".bookmark-row .row-open").focus();
	await page.keyboard.press("Enter");
	await expect.poll(() => acts(calls, "open").length).toBe(1);
	await page.keyboard.press("Space");
	await expect.poll(() => acts(calls, "open").length).toBe(2);
});

for (const height of [480, 1000]) {
	test(`the ${height}-pixel window keeps names in contiguous 48-pixel rows`, async ({
		page,
	}, info): Promise<void> => {
		await page.setViewportSize({ width: 1180, height });
		await library(page);
		await page.screenshot({ path: info.outputPath("folder-density.png") });
		const rows = await page
			.locator(".list > li:not(.spacer)")
			.evaluateAll(
				(
					elements,
				): readonly { readonly top: number; readonly height: number }[] =>
					elements.map((element) => {
						const box = element.getBoundingClientRect();
						return { top: box.top, height: box.height };
					}),
			);
		for (const [index, row] of rows.entries()) {
			expect(row.height).toBe(48);
			const previous = rows[index - 1];
			if (previous !== undefined)
				expect(row.top).toBe(previous.top + previous.height);
		}
		for (const target of await page.locator(".list .row-open").all())
			await expect(target).toHaveCSS("height", "48px");
	});
}
