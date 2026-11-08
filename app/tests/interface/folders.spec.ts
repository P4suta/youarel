import { readFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";
import axe from "axe-core";
import type { View } from "../../ui/types.js";
import { layoutOverflow } from "./design.js";
import { BOOKMARK, INITIAL, library } from "./fixture.js";

const FOLDER = "00000000000000000000000000000002";
const siteIconView: View = {
	...INITIAL,
	listing: {
		...INITIAL.listing,
		rows: INITIAL.listing.rows.map((row) =>
			row.kind === "shortcut"
				? { ...row, item: { ...BOOKMARK, icon: "site" } }
				: row,
		),
	},
};

for (const broken of [false, true]) {
	test(`a ${broken ? "broken" : "missing"} cached image keeps its link fallback`, async ({
		page,
	}): Promise<void> => {
		await library(page, {
			view: siteIconView,
			icons: broken ? { site: "data:image/png;base64,AA==" } : {},
		});
		await expect(page.locator(".site-icon .glyph")).toBeVisible();
		await expect(page.locator(".site-icon img")).toBeHidden();
	});
}

test("a decoded site image replaces the link fallback", async ({
	page,
}): Promise<void> => {
	const image = await readFile(
		new URL("../../icons/32x32.png", import.meta.url),
	);
	await library(page, {
		view: siteIconView,
		icons: { site: `data:image/png;base64,${image.toString("base64")}` },
	});
	await expect(page.locator(".site-icon img")).toBeVisible();
	await expect(page.locator(".site-icon .glyph")).toBeHidden();
});

test("every entry has a conventional type icon when no site icon is available", async ({
	page,
}): Promise<void> => {
	await library(page);
	await expect(page.locator(".folder-row .row-open use")).toHaveAttribute(
		"href",
		"#icon-folder",
	);
	await expect(page.locator(".bookmark-row .row-open use")).toHaveAttribute(
		"href",
		"#icon-link",
	);
	await expect(page.locator(".system-folder .row-open use")).toHaveAttribute(
		"href",
		"#icon-trash",
	);
});

test("a long URL stays out of the list and remains complete in bookmark details", async ({
	page,
}): Promise<void> => {
	const address = `https://mail.google.com/mail/u/0/#search/${"query".repeat(1000)}`;
	await library(page, {
		view: {
			...INITIAL,
			listing: {
				...INITIAL.listing,
				rows: INITIAL.listing.rows.map((row) =>
					row.kind === "shortcut"
						? { ...row, item: { ...BOOKMARK, link: address } }
						: row,
				),
			},
		},
	});
	await expect(
		page.locator("#content").getByText(address, { exact: true }),
	).toHaveCount(0);
	await expect(page.locator(".favicon.tile, .favicon.blank")).toHaveCount(0);
	await page
		.locator("#content")
		.getByText(BOOKMARK.title, { exact: true })
		.click({ button: "right" });
	await page.getByRole("menuitem", { name: "Edit…", exact: true }).click();
	await expect(page.locator("#edit-link")).toHaveText(address);
	await expect(page.locator("#edit-added")).toContainText("Added");
	expect(await layoutOverflow(page)).toEqual([]);
});

test("a double-click on a bookmark name opens its URL once", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	await page
		.getByRole("button", { name: `Open ${BOOKMARK.title}`, exact: true })
		.dblclick({ delay: 100 });
	await expect
		.poll(
			() =>
				calls.filter(
					(call) =>
						call.command === "act" &&
						typeof call.args.intent === "object" &&
						call.args.intent !== null &&
						"kind" in call.args.intent &&
						call.args.intent.kind === "open",
				).length,
		)
		.toBe(1);
});

test("the library presents names without a sidebar or metadata columns", async ({
	page,
}): Promise<void> => {
	await library(page);
	await expect(
		page.locator("aside, #sidebar-toggle, #sidebar-drawer"),
	).toHaveCount(0);
	await expect(page.getByText("Collections", { exact: true })).toHaveCount(0);
	await expect(page.getByText("Added", { exact: true })).toHaveCount(0);
	await expect(
		page.locator("#content").getByText(BOOKMARK.link, { exact: true }),
	).toHaveCount(0);
	await expect(
		page.getByRole("button", { name: "Open Research", exact: true }),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Open Trash", exact: true }),
	).toBeVisible();
	await expect(page.locator("#trail")).toBeHidden();
});

test("an empty folder opens with native keyboard activation", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	const folder = page.getByRole("button", {
		name: "Open Research",
		exact: true,
	});
	await folder.focus();
	await page.keyboard.press("Enter");
	expect(calls).toContainEqual({
		command: "act",
		args: { intent: { kind: "open", id: FOLDER } },
	});
});

test("a double-click in the blank area of a folder row opens it after selection redraws", async ({
	page,
}): Promise<void> => {
	const calls = await library(page);
	const folder = page.getByRole("button", {
		name: "Open Research",
		exact: true,
	});
	const box = await folder.boundingBox();
	if (box === null)
		throw new Error("The folder needs a visible activation target.");
	await folder.dblclick({
		position: { x: box.width - 16, y: box.height / 2 },
		delay: 100,
	});
	await expect
		.poll(
			() =>
				calls.filter(
					(call) =>
						call.command === "act" &&
						typeof call.args.intent === "object" &&
						call.args.intent !== null &&
						"kind" in call.args.intent &&
						call.args.intent.kind === "open",
				).length,
		)
		.toBe(1);
});

for (const width of [1180, 320]) {
	test(`the ${width}-pixel folder path wraps and returns to an ancestor`, async ({
		page,
	}, info): Promise<void> => {
		await page.setViewportSize({ width, height: 760 });
		const calls = await library(page, {
			view: {
				...INITIAL,
				scope: { kind: "collection", id: "notes" },
				here: { id: "notes", path: "Reading / Notes" },
				trail: [
					{
						id: FOLDER,
						name: "Reading material and reference documents",
						depth: 0,
					},
					{ id: "notes", name: "Notes and bookmarked research", depth: 1 },
				],
			},
		});
		if (width === 320) {
			await page.addStyleTag({ content: "html { font-size: 200%; }" });
			await page.evaluate(
				async (): Promise<void> =>
					new Promise((resolve) =>
						requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
					),
			);
		}
		const path = page.getByRole("navigation", { name: "Path", exact: true });
		await expect(path.locator("[aria-current='page']")).toHaveText(
			"Notes and bookmarked research",
		);
		expect(await layoutOverflow(page)).toEqual([]);
		await page.addScriptTag({ content: axe.source });
		expect(
			await page.evaluate(
				async (): Promise<string[]> =>
					(await window.axe.run(document)).violations.map((entry) => entry.id),
			),
		).toEqual([]);
		await page.screenshot({ path: info.outputPath("bookmark-folders.png") });
		await path
			.getByRole("button", {
				name: "Reading material and reference documents",
				exact: true,
			})
			.click();
		expect(calls).toContainEqual({
			command: "act",
			args: {
				intent: { kind: "scope", scope: { kind: "collection", id: FOLDER } },
			},
		});
	});
}
