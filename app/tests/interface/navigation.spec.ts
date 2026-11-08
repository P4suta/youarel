import { expect, test } from "@playwright/test";
import { library } from "./fixture.js";

test("rows omit checkboxes, disclosures, counts, and repeated type labels", async ({
	page,
}): Promise<void> => {
	await library(page);
	await expect(page.getByRole("checkbox")).toHaveCount(0);
	await expect(
		page.locator(
			".disclosure, .disclosure-spacer, .folder-row .count, .kind-label",
		),
	).toHaveCount(0);
});

test("selecting Trash clears the bookmark selection before Delete", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	const bookmark = page.getByRole("button", {
		name: "Open Reference notes",
		exact: true,
	});
	await bookmark.click({ modifiers: ["ControlOrMeta"] });
	await expect(bookmark).toHaveAccessibleDescription("Bookmark Selected");
	await page.getByRole("button", { name: "Open Trash", exact: true }).click();
	await expect(bookmark).toHaveAccessibleDescription("Bookmark Not selected");
	await page.keyboard.press("Delete");
	expect(
		calls.some(
			(call) =>
				call.command === "act" &&
				typeof call.args.intent === "object" &&
				call.args.intent !== null &&
				"kind" in call.args.intent &&
				call.args.intent.kind === "trash",
		),
	).toBe(false);
});

test("Trash is reached as a folder and its breadcrumb returns to the root", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	const trash = page.getByRole("button", { name: "Open Trash", exact: true });
	await trash.focus();
	await page.keyboard.press("Enter");
	await expect(
		page
			.getByRole("navigation", { name: "Path", exact: true })
			.locator("[aria-current='page']"),
	).toHaveText("Trash");
	await expect(
		page.getByRole("heading", { name: "Name", exact: true }),
	).toBeVisible();
	await expect(page.locator("#location button")).toHaveCount(1);
	await page
		.getByRole("navigation", { name: "Path", exact: true })
		.getByRole("button", { name: "Bookmarks", exact: true })
		.click();
	await expect(
		page.getByRole("button", { name: "Open Trash", exact: true }),
	).toBeVisible();
	expect(calls).toContainEqual({
		command: "act",
		args: { intent: { kind: "scope", scope: { kind: "all" } } },
	});
});
