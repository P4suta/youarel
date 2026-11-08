import { defineConfig } from "@playwright/test";

export default defineConfig({
	testDir: "./tests/interface",
	outputDir: "../target/interface",
	fullyParallel: false,
	workers: 1,
	timeout: 20_000,
	expect: { timeout: 4000 },
	reporter: "line",
	use: {
		browserName: "chromium",
		viewport: { width: 1180, height: 760 },
		screenshot: "only-on-failure",
	},
});
