import { expect, test } from "@playwright/test";
import { BOOKMARK, INITIAL, library } from "./fixture.js";

test("the root has a static name heading and no duplicated command controls", async ({
	page,
}): Promise<void> => {
	await library(page);
	await expect(
		page.getByRole("heading", { name: "Name", exact: true }),
	).toBeVisible();
	await expect(page.getByRole("searchbox")).toHaveCount(0);
	await expect(page.locator("#main > header").getByRole("button")).toHaveCount(
		0,
	);
	await expect(
		page.getByRole("button", { name: "Name", exact: true }),
	).toHaveCount(0);
	for (const row of await page
		.locator("#content")
		.getByRole("listitem")
		.all()) {
		await expect(row.getByRole("button")).toHaveCount(1);
	}
	await expect(
		page.getByRole("button", { name: "Bookmarks", exact: true }),
	).toHaveCount(0);
	await expect(page.locator("#content")).toBeFocused();
});

test("a row context menu works from the keyboard and restores its row focus", async ({
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
	await page.keyboard.press("Escape");
	await expect(bookmark).toBeFocused();
});

test("the add sheet saves in the current directory without a folder picker", async ({
	page,
}): Promise<void> => {
	const parent = { id: "00000000000000000000000000000002", path: "Research" };
	const calls = await library(page, {
		view: {
			...INITIAL,
			scope: { kind: "collection", id: parent.id },
			here: parent,
			trail: [{ id: parent.id, name: parent.path, depth: 0 }],
		},
	});
	await page
		.locator("#content")
		.click({ button: "right", position: { x: 100, y: 400 } });
	await page
		.getByRole("menuitem", { name: "New Bookmark…", exact: true })
		.click();
	const dialog = page.getByRole("dialog", { name: "Add a link", exact: true });
	await expect(dialog.getByRole("combobox")).toHaveCount(0);
	await dialog.getByLabel("URL", { exact: true }).fill(BOOKMARK.link);
	await dialog.getByRole("button", { name: "Add", exact: true }).click();
	await expect(dialog).not.toBeVisible();
	expect(calls.find((call) => call.command === "add")?.args).toMatchObject({
		collection: parent.id,
	});
	expect(calls.some((call) => call.command === "choices")).toBe(false);
});

test("the keyboard can create a bookmark from the empty current folder", async ({
	page,
}): Promise<void> => {
	await library(page, {
		view: {
			...INITIAL,
			scope: { kind: "collection", id: "empty" },
			here: { id: "empty", path: "Empty" },
			trail: [{ id: "empty", name: "Empty", depth: 0 }],
			listing: { ...INITIAL.listing, total: 0, folders: 0, rows: [] },
		},
	});
	await page.locator("#content").focus();
	await page.keyboard.press("Shift+F10");
	await expect(
		page.getByRole("menuitem", { name: "New Bookmark…", exact: true }),
	).toBeFocused();
	await page.keyboard.press("Enter");
	await expect(page.getByLabel("URL", { exact: true })).toBeFocused();
	await page.keyboard.press("Escape");
	await expect(page.locator("#content")).toBeFocused();
});

test("native menu commands follow the focused bookmark and close back to its name", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	const bookmark = page.getByRole("button", {
		name: `Open ${BOOKMARK.title}`,
		exact: true,
	});
	await bookmark.focus();
	await expect
		.poll(
			() =>
				calls.filter((call) => call.command === "history_menu").at(-1)?.args,
		)
		.toMatchObject({
			state: {
				enabled: expect.arrayContaining(["edit-bookmark", "copy-url"]),
			},
		});
	await page.evaluate((): void => {
		window.dispatchEvent(
			new CustomEvent("youarel:command", { detail: "edit-bookmark" }),
		);
	});
	const dialog = page.getByRole("dialog", {
		name: "Edit bookmark",
		exact: true,
	});
	await expect(dialog.getByLabel("Title", { exact: true })).toBeFocused();
	await expect
		.poll(
			() =>
				calls.filter((call) => call.command === "history_menu").at(-1)?.args,
		)
		.toMatchObject({ state: { enabled: [] } });
	await page.keyboard.press("Escape");
	await expect(bookmark).toBeFocused();
});

test("Trash does not offer bookmark or folder creation", async ({
	page,
}): Promise<void> => {
	await library(page, {
		view: { ...INITIAL, scope: { kind: "trash" }, trash: 1 },
	});
	await page.locator("#content").focus();
	await page.keyboard.press("Shift+F10");
	await expect(
		page.getByRole("menuitem", { name: "Empty Trash…", exact: true }),
	).toBeFocused();
	await expect(
		page.getByRole("menuitem", { name: "New Bookmark…", exact: true }),
	).toHaveCount(0);
	await page.keyboard.press("Escape");
	await page.keyboard.press("ControlOrMeta+n");
	await page.keyboard.press("ControlOrMeta+Shift+n");
	await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("native deletion of a focused trashed folder explains that its contents are deleted", async ({
	page,
}): Promise<void> => {
	await library(page, {
		view: { ...INITIAL, scope: { kind: "trash" }, trash: 1 },
	});
	await page.locator(".folder-row .row-open").focus();
	await page.evaluate((): void => {
		window.dispatchEvent(
			new CustomEvent("youarel:command", { detail: "trash-selection" }),
		);
	});
	const dialog = page.getByRole("dialog", {
		name: "Delete 1 folder immediately?",
		exact: true,
	});
	await expect(dialog).toContainText("The items and their contents");
	await expect(
		dialog.getByRole("button", { name: "Cancel", exact: true }),
	).toBeFocused();
});
