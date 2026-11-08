import { expect, test } from "@playwright/test";
import axe from "axe-core";
import { BOOKMARK, library, newBookmark, rowMenu } from "./fixture.js";

test("the rendered library has accessible semantics", async ({
	page,
}): Promise<void> => {
	await library(page);
	await expect(page.locator("#content")).toBeFocused();
	await page.addScriptTag({ content: axe.source });
	const violations = await page.evaluate(async (): Promise<string[]> => {
		const result = await window.axe.run(document, {
			runOnly: [
				"wcag2a",
				"wcag2aa",
				"wcag21a",
				"wcag21aa",
				"wcag22aa",
				"best-practice",
			],
		});
		return result.violations.map(
			(entry) =>
				`${entry.id}: ${entry.nodes.map((node) => node.target.join(" ")).join(", ")}`,
		);
	});
	expect(violations).toEqual([]);
});

test("a named add dialog traps focus and returns it to the current folder", async ({
	page,
}): Promise<void> => {
	await library(page);
	const opener = page.locator("#content");
	await newBookmark(page);
	const dialog = page.getByRole("dialog", { name: "Add a link" });
	await expect(dialog).toBeVisible();
	await page.keyboard.press("Shift+Tab");
	await expect(dialog.locator(":focus")).toHaveCount(1);
	await page.keyboard.press("Escape");
	await expect(dialog).not.toBeVisible();
	await expect(opener).toBeFocused();
});

test("invalid addresses keep a readable error beside the field", async ({
	page,
}): Promise<void> => {
	await library(page, { refuse: "add" });
	await newBookmark(page);
	const address = page.getByLabel("URL", { exact: true });
	await address.fill("javascript:alert(1)");
	await page.locator("#add-submit").click();
	await expect(address).toHaveAttribute("aria-invalid", "true");
	await expect(address).toBeFocused();
	await expect(page.locator("#add-link-error")).toBeVisible();
});

test("bookmark actions work without a pointer or context click", async ({
	page,
}): Promise<void> => {
	await library(page);
	const bookmark = page.getByRole("button", {
		name: `Open ${BOOKMARK.title}`,
		exact: true,
	});
	await bookmark.focus();
	await page.keyboard.press("Shift+F10");
	await expect(
		page.getByRole("menuitem", { name: "Open", exact: true }),
	).toBeFocused();
	await page.keyboard.press("ArrowDown");
	await expect(
		page.getByRole("menuitem", { name: "Copy URL", exact: true }),
	).toBeFocused();
	await page.keyboard.press("Escape");
	await expect(bookmark).toBeFocused();
});

test("forms remain usable at a narrow width and enlarged text", async ({
	page,
}): Promise<void> => {
	await page.setViewportSize({ width: 320, height: 680 });
	await library(page);
	await page.addStyleTag({ content: "html { font-size: 200% !important; }" });
	await newBookmark(page);
	const dialog = page.getByRole("dialog", { name: "Add a link" });
	await expect(dialog).toBeVisible();
	const overflow = await dialog.evaluate(
		(element): boolean => element.scrollWidth > element.clientWidth + 1,
	);
	expect(overflow).toBe(false);
	const cancel = dialog.getByRole("button", { name: "Cancel", exact: true });
	await cancel.focus();
	await expect(cancel).toBeInViewport();
});

declare global {
	interface Window {
		axe: typeof axe;
	}
}

test("every modal declares its native accessibility contract", async ({
	page,
}): Promise<void> => {
	await library(page);
	const dialogs = page.locator("dialog");
	expect(await dialogs.count()).toBeGreaterThan(0);
	for (const dialog of await dialogs.all()) {
		await expect(dialog).toHaveJSProperty("tagName", "DIALOG");
	}
	await page.addScriptTag({ content: axe.source });
	for (const entry of [
		{ action: "Choose emoji…", name: "Emoji for Research" },
		{ action: "Rename…", name: "Rename folder" },
		{ action: "Move to Folder…", name: "Move folder" },
	]) {
		await rowMenu(page, "Research");
		await page
			.getByRole("menuitem", { name: entry.action, exact: true })
			.click();
		const dialog = page.getByRole("dialog", { name: entry.name, exact: true });
		await expect(dialog).toBeVisible();
		await expect(dialog).toHaveAttribute("aria-label", entry.name);
		await expect(dialog).not.toHaveAttribute("aria-labelledby");
		expect(
			await dialog.evaluate((element): boolean => element.matches(":modal")),
		).toBe(true);
		const violations = await page.evaluate(
			async (): Promise<readonly string[]> =>
				(await window.axe.run(document)).violations.map(
					(violation) => violation.id,
				),
		);
		expect(violations).toEqual([]);
		await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
	}
});
