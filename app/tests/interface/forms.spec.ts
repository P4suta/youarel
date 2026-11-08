import { expect, type Page, test } from "@playwright/test";
import axe from "axe-core";
import type { Choice } from "../../ui/types.js";
import { layoutOverflow } from "./design.js";
import { BOOKMARK, library } from "./fixture.js";

const choices: readonly Choice[] = Array.from(
	{ length: 12 },
	(_, index): Choice => ({
		id: String(index + 10).padStart(32, "0"),
		path: `Collection ${String(index + 1).padStart(2, "0")}`,
	}),
);

async function edit(page: Page): Promise<void> {
	await page
		.locator("#content")
		.getByText(BOOKMARK.title, { exact: true })
		.click({ button: "right" });
	await page.getByRole("menuitem", { name: "Edit…", exact: true }).click();
}

test("edit fields follow one reading column with consistent control geometry", async ({
	page,
}, info): Promise<void> => {
	await library(page);
	await edit(page);
	const dialog = page.getByRole("dialog", { name: "Edit bookmark" });
	const fields = [
		dialog.getByRole("textbox", { name: "Title", exact: true }),
		dialog.getByRole("combobox", { name: "Folder", exact: true }),
		dialog.getByRole("textbox", { name: "Alias", exact: true }),
		dialog.getByRole("textbox", { name: "Note", exact: true }),
	];
	const bounds = await Promise.all(fields.map((field) => field.boundingBox()));
	const first = bounds[0];
	if (first === null || first === undefined)
		throw new Error("The title must be visible.");
	for (const [index, bound] of bounds.entries()) {
		if (bound === null) throw new Error("Every edit field must be visible.");
		expect(Math.abs(bound.x - first.x)).toBeLessThanOrEqual(1);
		expect(Math.abs(bound.width - first.width)).toBeLessThanOrEqual(1);
		if (index < fields.length - 1)
			expect(Math.abs(bound.height - first.height)).toBeLessThanOrEqual(1);
		const previous = bounds[index - 1];
		if (previous !== null && previous !== undefined)
			expect(bound.y).toBeGreaterThan(previous.y + previous.height);
	}
	await expect(dialog.getByRole("searchbox")).toHaveCount(0);
	await expect(
		dialog.getByRole("combobox", { name: "Folder", exact: true }),
	).toHaveCount(1);
	await page.screenshot({ path: info.outputPath("edit-form.png") });
});

test("a small collection set uses one native selection control", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	await edit(page);
	const dialog = page.getByRole("dialog", { name: "Edit bookmark" });
	const collection = dialog.getByRole("combobox", {
		name: "Folder",
		exact: true,
	});
	await expect(collection).toHaveJSProperty("tagName", "SELECT");
	await collection.selectOption({ label: "Research" });
	await dialog.getByRole("button", { name: "Save", exact: true }).click();
	await expect(dialog).not.toBeVisible();
	expect(
		calls.filter((call) => call.command === "act").at(-1)?.args,
	).toMatchObject({
		intent: {
			kind: "edit",
			fields: { collection: "00000000000000000000000000000002" },
		},
	});
});

test("many collections use one searchable combobox with keyboard selection", async ({
	page,
}): Promise<void> => {
	const calls = await library(page, { choices });
	await edit(page);
	const dialog = page.getByRole("dialog", { name: "Edit bookmark" });
	const collection = dialog.getByRole("combobox", {
		name: "Folder",
		exact: true,
	});
	await expect(collection).toHaveJSProperty("tagName", "INPUT");
	await collection.fill("Collection 12");
	const found = page.getByRole("option", {
		name: "Collection 12",
		exact: true,
	});
	await expect(found).toBeVisible();
	await expect(collection).toHaveAttribute(
		"aria-activedescendant",
		(await found.getAttribute("id")) ?? "",
	);
	await page.keyboard.press("Enter");
	await expect(collection).toHaveValue("Collection 12");
	await expect(collection).toHaveAttribute("aria-expanded", "false");
	await expect(dialog).toBeVisible();
	await dialog.getByRole("button", { name: "Save", exact: true }).click();
	await expect(dialog).not.toBeVisible();
	expect(
		calls.filter((call) => call.command === "act").at(-1)?.args,
	).toMatchObject({
		intent: { kind: "edit", fields: { collection: choices.at(-1)?.id } },
	});
});

test("Escape cancels a collection search before closing its form", async ({
	page,
}): Promise<void> => {
	await library(page, { choices });
	await edit(page);
	const dialog = page.getByRole("dialog", { name: "Edit bookmark" });
	const collection = dialog.getByRole("combobox", {
		name: "Folder",
		exact: true,
	});
	await expect(collection).toHaveJSProperty("tagName", "INPUT");
	await collection.fill("Collection");
	await expect(
		page.getByRole("option", { name: "Collection 01", exact: true }),
	).toBeVisible();
	await page.keyboard.press("ArrowDown");
	await page.keyboard.press("Escape");
	await expect(dialog).toBeVisible();
	await expect(collection).toHaveValue("Bookmarks");
	await expect(collection).toBeFocused();
	await page.keyboard.press("Escape");
	await expect(dialog).not.toBeVisible();
});

test("the collection popup remains reachable with narrow, enlarged text", async ({
	page,
}, info): Promise<void> => {
	await page.setViewportSize({ width: 320, height: 680 });
	await library(page, { choices });
	await page.addStyleTag({ content: "html { font-size: 200%; }" });
	await edit(page);
	const collection = page.getByRole("combobox", {
		name: "Folder",
		exact: true,
	});
	await expect(collection).toHaveJSProperty("tagName", "INPUT");
	await collection.fill("Collection 12");
	const found = page.getByRole("option", {
		name: "Collection 12",
		exact: true,
	});
	await expect(found).toBeInViewport();
	expect(await layoutOverflow(page)).toEqual([]);
	await page.addScriptTag({ content: axe.source });
	expect(
		await page.evaluate(async () =>
			(await window.axe.run(document)).violations.map((entry) => entry.id),
		),
	).toEqual([]);
	await page.screenshot({ path: info.outputPath("collection-search.png") });
	await found.click();
	await expect(collection).toHaveValue("Collection 12");
});
