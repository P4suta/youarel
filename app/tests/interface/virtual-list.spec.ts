import { expect, test } from "@playwright/test";
import type { RowView } from "../../ui/types.js";
import { BOOKMARK, INITIAL, library } from "./fixture.js";

const TOTAL = 120;
const rows: readonly RowView[] = Array.from(
	{ length: TOTAL },
	(_, index): RowView => ({
		kind: "shortcut",
		item: {
			...BOOKMARK,
			id: String(index + 1).padStart(32, "0"),
			title: `Bookmark ${String(index + 1).padStart(3, "0")}`,
		},
		selected: false,
		cut: false,
	}),
);
const view = {
	...INITIAL,
	listing: { ...INITIAL.listing, total: TOTAL, rows, folders: 0 },
};

test("the virtual list exposes logical item positions", async ({
	page,
}): Promise<void> => {
	await library(page, { view });
	const list = page.getByRole("list", { name: "Folder contents", exact: true });
	await expect(list).toBeVisible();
	await expect(
		page
			.getByRole("button", { name: "Open Bookmark 001", exact: true })
			.locator("xpath=ancestor::li"),
	).toHaveAttribute("aria-posinset", "2");
	await page.locator("#content").evaluate((element): void => {
		element.scrollTop = element.scrollHeight;
	});
	const last = page.getByRole("button", {
		name: "Open Bookmark 120",
		exact: true,
	});
	await expect(last).toBeVisible();
	await expect(last.locator("xpath=ancestor::li")).toHaveAttribute(
		"aria-posinset",
		String(TOTAL + 1),
	);
	await expect(last.locator("xpath=ancestor::li")).toHaveAttribute(
		"aria-setsize",
		String(TOTAL + 1),
	);
});

test("keyboard focus survives virtual scrolling and reaches the following row", async ({
	page,
}): Promise<void> => {
	await library(page, { view });
	const number = Number(
		await page.locator(".bookmark-row").last().getAttribute("data-id"),
	);
	expect(number).toBeLessThan(TOTAL);
	const name = `Bookmark ${String(number).padStart(3, "0")}`;
	const open = page.getByRole("button", {
		name: `Open ${name}`,
		exact: true,
	});
	await open.focus();
	await page.evaluate(
		async (): Promise<void> =>
			new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
			),
	);
	await expect(open).toBeFocused();
	await page.keyboard.press("Shift+F10");
	await expect(
		page.getByRole("menuitem", { name: "Open", exact: true }),
	).toBeFocused();
	await page.keyboard.press("Escape");
	await expect(open).toBeFocused();
	await page.keyboard.press("Tab");
	await expect(
		page.getByRole("button", {
			name: `Open Bookmark ${String(number + 1).padStart(3, "0")}`,
			exact: true,
		}),
	).toBeFocused();
});
