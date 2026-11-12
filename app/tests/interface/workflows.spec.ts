import { expect, type Page, test } from "@playwright/test";
import type { Intent } from "../../ui/types.js";
import {
	BOOKMARK,
	type Call,
	INITIAL,
	library,
	newBookmark,
	rowMenu,
} from "./fixture.js";

function intents(
	calls: readonly Call[],
	kind: Intent["kind"],
): readonly Call[] {
	return calls.filter(
		(call) =>
			call.command === "act" &&
			typeof call.args.intent === "object" &&
			call.args.intent !== null &&
			"kind" in call.args.intent &&
			call.args.intent.kind === kind,
	);
}

for (const key of ["Delete", "ControlOrMeta+Backspace"]) {
	test(`${key} waits for a pending selection before moving it to Trash`, async ({
		page,
	}): Promise<void> => {
		let release!: () => void;
		const ready = new Promise<void>((resolve) => {
			release = resolve;
		});
		const calls = await library(page, {
			pendingIntent: { kind: "select", ready },
		});
		const bookmark = page.locator(".bookmark-row .row-open");
		await bookmark.click();
		await expect.poll(() => intents(calls, "select").length).toBe(1);
		await expect(bookmark).toHaveAccessibleDescription("Bookmark Not selected");
		await page.keyboard.press(key);
		release();
		await expect(bookmark).toHaveAccessibleDescription("Bookmark Selected");
		await expect.poll(() => intents(calls, "trash").length).toBe(1);
	});
}

test("Delete respects a pending selection cancellation", async ({
	page,
}): Promise<void> => {
	let release!: () => void;
	const ready = new Promise<void>((resolve) => {
		release = resolve;
	});
	const calls = await library(page, {
		pendingIntent: { kind: "cancel", ready },
	});
	const bookmark = page.locator(".bookmark-row .row-open");
	await bookmark.click();
	await expect(bookmark).toHaveAccessibleDescription("Bookmark Selected");
	await page.keyboard.press("Escape");
	await expect.poll(() => intents(calls, "cancel").length).toBe(1);
	await page.keyboard.press("Delete");
	release();
	await page.keyboard.press("ControlOrMeta+z");
	await expect.poll(() => intents(calls, "undo").length).toBe(1);
	await expect(bookmark).toHaveAccessibleDescription("Bookmark Not selected");
	expect(intents(calls, "trash")).toHaveLength(0);
});

test("permanent deletion confirms the selection after a pending change", async ({
	page,
}): Promise<void> => {
	let release!: () => void;
	const ready = new Promise<void>((resolve) => {
		release = resolve;
	});
	const calls = await library(page, {
		pendingIntent: { kind: "select", ready },
	});
	await page.locator(".bookmark-row .row-open").click();
	await expect.poll(() => intents(calls, "select").length).toBe(1);
	await page.keyboard.press("ControlOrMeta+Alt+Backspace");
	release();
	const confirmation = page.getByRole("dialog", {
		name: "Delete this shortcut immediately?",
		exact: true,
	});
	await expect(confirmation).toBeVisible();
	expect(intents(calls, "remove")).toHaveLength(0);
	await confirmation
		.getByRole("button", { name: "Delete", exact: true })
		.click();
	await expect.poll(() => intents(calls, "remove").length).toBe(1);
	expect(intents(calls, "trash")).toHaveLength(0);
});

async function bookmarkActions(page: Page): Promise<void> {
	await rowMenu(page, BOOKMARK.title);
}

test("a failed add preserves the draft and focuses a persistent error", async ({
	page,
}): Promise<void> => {
	const calls = await library(page, { refuse: "add" });
	await newBookmark(page);
	await page.getByLabel("URL", { exact: true }).fill(BOOKMARK.link);
	await page
		.getByRole("textbox", { name: "Title", exact: true })
		.fill("Keep my draft");
	await page.locator("#add-submit").click();
	await expect(page.locator("#add-error")).toBeFocused();
	await expect(page.locator("#add-error")).toContainText("Try again");
	await expect(
		page.getByRole("textbox", { name: "Title", exact: true }),
	).toHaveValue("Keep my draft");
	await expect(page.locator("#add-submit")).not.toHaveAttribute(
		"aria-disabled",
		"true",
	);
	expect(calls.filter((call) => call.command === "add")).toHaveLength(1);
});

test("a failed edit keeps the edited fields available for retry", async ({
	page,
}): Promise<void> => {
	await library(page, { refuseIntent: "edit" });
	await bookmarkActions(page);
	await page.getByRole("menuitem", { name: "Edit…", exact: true }).click();
	const dialog = page.getByRole("dialog", { name: "Edit bookmark" });
	await dialog.getByLabel("Title", { exact: true }).fill("Revised title");
	await dialog
		.getByLabel("Note", { exact: true })
		.fill("Keep the revised note");
	await dialog.getByRole("button", { name: "Save", exact: true }).click();
	await expect(page.locator("#edit-error")).toBeFocused();
	await expect(dialog.getByLabel("Title", { exact: true })).toHaveValue(
		"Revised title",
	);
	await expect(dialog.getByLabel("Note", { exact: true })).toHaveValue(
		"Keep the revised note",
	);
	await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
	await expect(
		page.getByRole("button", {
			name: `Open ${BOOKMARK.title}`,
			exact: true,
		}),
	).toBeFocused();
});

for (const form of ["add", "edit"] as const) {
	test(`the ${form} sheet owns its error without a duplicate notification`, async ({
		page,
	}): Promise<void> => {
		await library(page, {
			errorView: true,
			...(form === "add" ? { refuse: "add" } : { refuseIntent: "edit" }),
		});
		if (form === "add") {
			await newBookmark(page);
			await page.getByLabel("URL", { exact: true }).fill(BOOKMARK.link);
			await page.locator("#add-submit").click();
		} else {
			await bookmarkActions(page);
			await page.getByRole("menuitem", { name: "Edit…", exact: true }).click();
			await page.getByRole("button", { name: "Save", exact: true }).click();
		}
		await expect(page.locator(`#${form}-error`)).toBeFocused();
		await expect(page.locator("#notice")).not.toBeVisible();
		await page.keyboard.press("Escape");
		await expect(page.locator("#notice")).not.toBeVisible();
	});
}

test("late clipboard content cannot replace a reopened form or steal focus", async ({
	page,
}): Promise<void> => {
	let settle: ((value: string | null) => void) | undefined;
	const clipboard = new Promise<string | null>((resolve) => {
		settle = resolve;
	});
	await library(page, { clipboard });
	const opener = page.locator("#content");
	await newBookmark(page);
	await page.keyboard.press("Escape");
	await expect(opener).toBeFocused();
	await newBookmark(page);
	const draft = "https://other.example/bookmark";
	await page.getByLabel("URL", { exact: true }).fill(draft);
	const title = page.getByRole("textbox", { name: "Title", exact: true });
	await title.focus();
	settle?.(BOOKMARK.link);
	await expect(title).toBeFocused();
	await expect(page.getByLabel("URL", { exact: true })).toHaveValue(draft);
});

test("modified name clicks select without opening the bookmark", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	const selection = page.getByRole("button", {
		name: `Open ${BOOKMARK.title}`,
		exact: true,
	});
	await selection.click({ modifiers: ["ControlOrMeta"] });
	await expect(selection).toHaveAccessibleDescription("Bookmark Selected");
	await expect(selection).toBeFocused();
	await selection.click({ modifiers: ["ControlOrMeta"] });
	await expect(selection).toHaveAccessibleDescription("Bookmark Not selected");
	expect(
		calls.filter(
			(call) =>
				call.command === "act" &&
				typeof call.args.intent === "object" &&
				call.args.intent !== null &&
				"kind" in call.args.intent &&
				call.args.intent.kind === "open",
		),
	).toHaveLength(0);
});

test("menu navigation supports boundaries, typeahead, and leaving with Tab", async ({
	page,
}): Promise<void> => {
	await library(page);
	await bookmarkActions(page);
	await page.keyboard.press("End");
	await expect(
		page.getByRole("menuitem", { name: "Move to Trash", exact: true }),
	).toBeFocused();
	await page.keyboard.press("Home");
	await expect(
		page.getByRole("menuitem", { name: "Open", exact: true }),
	).toBeFocused();
	await page.keyboard.press("e");
	await expect(
		page.getByRole("menuitem", { name: "Edit…", exact: true }),
	).toBeFocused();
	await page.keyboard.press("Tab");
	await expect(page.getByRole("menu")).not.toBeVisible();
	await expect(page.locator("#menu :focus")).toHaveCount(0);
});

test("permanent deletion starts on Cancel and cancellation sends no deletion", async ({
	page,
}): Promise<void> => {
	const calls = await library(page, {
		view: {
			...INITIAL,
			scope: { kind: "trash" },
			trash: 1,
		},
	});
	await bookmarkActions(page);
	await page
		.getByRole("menuitem", { name: "Delete Immediately…", exact: true })
		.click();
	const dialog = page.getByRole("dialog", {
		name: "Delete this shortcut immediately?",
	});
	await expect(
		dialog.getByRole("button", { name: "Cancel", exact: true }),
	).toBeFocused();
	await page.keyboard.press("Enter");
	await expect(dialog).not.toBeVisible();
	expect(calls.filter((call) => call.command === "act")).toHaveLength(1);
});

test("a collection can move through visible controls without dragging", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	await rowMenu(page, "Research");
	await page
		.getByRole("menuitem", { name: "Move to Folder…", exact: true })
		.click();
	const dialog = page.getByRole("dialog", { name: "Move folder" });
	await expect(
		dialog.getByRole("combobox", { name: "Destination", exact: true }),
	).toBeFocused();
	await dialog
		.getByRole("combobox", { name: "Destination", exact: true })
		.selectOption("");
	await dialog.getByRole("button", { name: "Move", exact: true }).click();
	await expect(dialog).not.toBeVisible();
	expect(
		calls.some(
			(call) =>
				call.command === "act" &&
				typeof call.args.intent === "object" &&
				call.args.intent !== null &&
				"kind" in call.args.intent &&
				call.args.intent.kind === "file",
		),
	).toBe(true);
});

test("a narrow folder form restores focus to its folder name", async ({
	page,
}): Promise<void> => {
	await page.setViewportSize({ width: 320, height: 650 });
	await library(page);
	const actions = page.getByRole("button", {
		name: "Open Research",
		exact: true,
	});
	await rowMenu(page, "Research");
	await page.getByRole("menuitem", { name: "Rename…", exact: true }).click();
	const dialog = page.getByRole("dialog", {
		name: "Rename folder",
		exact: true,
	});
	await expect(dialog.getByLabel("Name", { exact: true })).toBeFocused();
	await expect(page.locator("dialog[open]")).toHaveCount(1);
	await page.keyboard.press("Escape");
	await expect(actions).toBeFocused();
});

test("an empty folder explains how to create its first bookmark", async ({
	page,
}): Promise<void> => {
	await library(page, {
		view: {
			...INITIAL,
			listing: { ...INITIAL.listing, total: 0, folders: 0, rows: [] },
		},
	});
	await expect(
		page.getByRole("heading", { name: "No bookmarks yet", exact: true }),
	).toBeVisible();
	await expect(page.locator(".empty")).toContainText("Shift+F10");
	await expect(page.locator("#results-status")).toHaveText(
		"No bookmarks in this folder.",
	);
	await expect(page.locator("#content")).not.toHaveAttribute("aria-live");
});

test("completion notifications disappear without moving focus or returning after refresh", async ({
	page,
}): Promise<void> => {
	await page.clock.install({ time: new Date("2026-11-08T00:00:00Z") });
	await page.clock.pauseAt(new Date("2026-11-08T00:01:00Z"));
	await library(page, {
		notice: { text: "The bookmark was saved.", error: false },
	});
	await page.clock.fastForward(4_999);
	await expect(page.locator("#notice-text")).toHaveText(
		"The bookmark was saved.",
	);
	await page.clock.fastForward(1);
	await expect(page.locator("#notice")).not.toBeVisible();
	await expect(page.locator("#content")).toBeFocused();
	await page.evaluate((): void => {
		window.dispatchEvent(new Event("focus"));
	});
	await expect(page.locator("#notice")).not.toBeVisible();
});

test("notification expiry waits while hovered or keyboard focused", async ({
	page,
}): Promise<void> => {
	await page.clock.install();
	await library(page, {
		notice: { text: "The bookmark was saved.", error: false },
	});
	await page.locator("#notice").hover();
	await page.clock.fastForward(60_000);
	await expect(page.locator("#notice")).toBeVisible();
	await page.locator("#notice-dismiss").focus();
	await page.mouse.move(1170, 0);
	await page.clock.fastForward(60_000);
	await expect(page.locator("#notice-dismiss")).toBeFocused();
	await page.locator("#content").focus();
	await page.clock.fastForward(5_000);
	await expect(page.locator("#notice")).not.toBeVisible();
});

test("a notification cannot expire a later progress message", async ({
	page,
}): Promise<void> => {
	await page.clock.install();
	await library(page, {
		notice: { text: "The bookmark was saved.", error: false },
	});
	await page.clock.fastForward(4_000);
	await page.evaluate((): void => {
		window.dispatchEvent(
			new CustomEvent("youarel:progress", {
				detail: { text: "Moving bookmarks", done: 1, total: 10 },
			}),
		);
	});
	await page.clock.fastForward(60_000);
	await expect(page.locator("#notice-text")).toHaveText("Moving bookmarks");
});

test("errors remain available and can be dismissed without losing folder focus", async ({
	page,
}): Promise<void> => {
	await page.clock.install();
	await library(page, {
		notice: {
			text: "The bookmark could not be saved. Try again.",
			error: true,
		},
	});
	await page.clock.fastForward(60_000);
	await expect(page.locator("#notice-text")).toContainText("Try again");
	await page
		.getByRole("button", { name: "Dismiss notification", exact: true })
		.click();
	await expect(page.locator("#notice")).not.toBeVisible();
	await expect(page.locator("#content")).toBeFocused();
});

test("a successful selection clears obsolete progress feedback", async ({
	page,
}): Promise<void> => {
	await library(page);
	await page.evaluate((): void => {
		window.dispatchEvent(
			new CustomEvent("youarel:progress", {
				detail: { text: "Moving bookmarks", done: 1, total: 10 },
			}),
		);
	});
	await expect(page.locator("#notice")).toBeVisible();
	await page.locator(".folder-row .row-open").click();
	await expect(page.locator("#notice")).not.toBeVisible();
});

test("a failed collection creation preserves the name for retry", async ({
	page,
}): Promise<void> => {
	await library(page, { refuseIntent: "newCollection" });
	await page.keyboard.press("ControlOrMeta+Shift+n");
	const dialog = page.getByRole("dialog", {
		name: "New folder",
		exact: true,
	});
	await dialog.getByLabel("Name", { exact: true }).fill("Reading");
	await dialog.getByRole("button", { name: "Create", exact: true }).click();
	await expect(dialog).toBeVisible();
	await expect(dialog.getByLabel("Name", { exact: true })).toHaveValue(
		"Reading",
	);
	await expect(page.locator("#ask-error")).toBeFocused();
});

test("a queued dialog close does not undo a later focus change", async ({
	page,
}): Promise<void> => {
	await library(page);
	await newBookmark(page);
	await page.evaluate((): void => {
		const dialog = document.getElementById("add-sheet");
		const next = document.querySelector(".system-folder .row-open");
		if (
			!(dialog instanceof HTMLDialogElement) ||
			!(next instanceof HTMLElement)
		)
			throw new Error("The fixture needs the add dialog and Trash folder.");
		dialog.close();
		next.focus();
	});
	await expect(
		page.getByRole("button", { name: "Open Trash", exact: true }),
	).toBeFocused();
});

test("an older close event cannot dismiss a reopened collection form", async ({
	page,
}): Promise<void> => {
	await library(page);
	await page.keyboard.press("ControlOrMeta+Shift+n");
	await page.evaluate(async (): Promise<void> => {
		const dialog = document.getElementById("ask-sheet");
		if (!(dialog instanceof HTMLDialogElement))
			throw new Error("The fixture needs the folder form.");
		dialog.close();
		window.dispatchEvent(
			new CustomEvent("youarel:command", { detail: "new-collection" }),
		);
		await new Promise<void>((resolve) =>
			requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
		);
	});
	await expect(
		page.getByRole("dialog", { name: "New folder", exact: true }),
	).toBeVisible();
	await expect(
		page.getByRole("textbox", { name: "Name", exact: true }),
	).toBeFocused();
});
