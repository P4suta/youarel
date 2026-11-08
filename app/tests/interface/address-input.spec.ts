import { expect, test } from "@playwright/test";
import { library, newBookmark } from "./fixture.js";

for (const [input, normalized] of [
	["google.com", "https://google.com/"],
	["www.google.com", "https://www.google.com/"],
	[
		"google.com/search?q=rust#results",
		"https://google.com/search?q=rust#results",
	],
	["例え.jp", "https://xn--r8jz45g.jp/"],
] as const) {
	test(`adding ${input} uses its completed address`, async ({
		page,
	}): Promise<void> => {
		const calls = await library(page, {
			normalizedLinks: { [input]: normalized },
		});
		await newBookmark(page);
		await page.getByLabel("URL", { exact: true }).fill(input);
		await page.locator("#add-submit").click();
		await expect(
			page.getByRole("dialog", { name: "Add a link", exact: true }),
		).not.toBeVisible();
		expect(calls.find((call) => call.command === "add")?.args).toMatchObject({
			link: normalized,
		});
	});
}

test("title preview uses the completed address", async ({
	page,
}): Promise<void> => {
	const calls = await library(page, {
		normalizedLinks: { "google.com": "https://google.com/" },
	});
	await newBookmark(page);
	await page.getByLabel("URL", { exact: true }).fill("google.com");
	await expect
		.poll(() => calls.filter((call) => call.command === "title"))
		.toEqual([{ command: "title", args: { link: "https://google.com/" } }]);
});

for (const source of ["clipboard", "drop"] as const) {
	test(`a domain from ${source} can be added`, async ({
		page,
	}): Promise<void> => {
		const calls = await library(page, {
			normalizedLinks: { "google.com": "https://google.com/" },
			...(source === "clipboard"
				? { clipboard: Promise.resolve("google.com") }
				: {}),
		});
		if (source === "clipboard") {
			await newBookmark(page);
		} else {
			await page.evaluate((): void => {
				const dataTransfer = new DataTransfer();
				dataTransfer.setData("text/plain", "google.com");
				document.dispatchEvent(
					new DragEvent("drop", {
						dataTransfer,
						bubbles: true,
						cancelable: true,
					}),
				);
			});
		}
		await expect(page.getByLabel("URL", { exact: true })).toHaveValue(
			"google.com",
		);
		await page.locator("#add-submit").click();
		await expect
			.poll(() => calls.filter((call) => call.command === "add"))
			.toHaveLength(1);
		expect(calls.find((call) => call.command === "add")?.args.link).toBe(
			"https://google.com/",
		);
	});
}

test("late address validation cannot submit a canceled or reopened draft", async ({
	page,
}): Promise<void> => {
	let settle: ((value: string | null) => void) | undefined;
	const normalized = new Promise<string | null>((resolve) => {
		settle = resolve;
	});
	const calls = await library(page, {
		normalizedLinks: { "google.com": normalized },
	});
	await newBookmark(page);
	await page.getByLabel("URL", { exact: true }).fill("google.com");
	await page.getByRole("textbox", { name: "Title", exact: true }).fill("Draft");
	await page.locator("#add-submit").click();
	await expect(page.locator("#add-submit")).toHaveAttribute(
		"aria-disabled",
		"true",
	);
	await page.keyboard.press("Escape");
	await newBookmark(page);
	await page.getByLabel("URL", { exact: true }).fill("https://other.example/");
	await page
		.getByRole("textbox", { name: "Title", exact: true })
		.fill("Other draft");
	settle?.("https://google.com/");
	await expect(page.locator("#add-submit")).not.toHaveAttribute(
		"aria-disabled",
		"true",
	);
	await page.locator("#add-submit").click();
	await expect(page.locator("#add-sheet")).not.toBeVisible();
	expect(calls.filter((call) => call.command === "add")).toHaveLength(1);
	expect(calls.find((call) => call.command === "add")?.args.link).toBe(
		"https://other.example/",
	);
});
