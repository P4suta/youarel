import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { parse as parseToml } from "@iarna/toml";
import { GitHub } from "release-please";
import { GenericJson } from "release-please/build/src/updaters/generic-json.js";
import { GenericToml } from "release-please/build/src/updaters/generic-toml.js";
import { coerceOption } from "release-please/build/src/util/coerce-option.js";
import { Version } from "release-please/build/src/version.js";

const ROOT = resolve(import.meta.dirname, "../..");
const TARGET_VERSION = Version.parse("0.7.4");

test("the proposal client's repository globs keep matching with the patched minimatch dependency", async (): Promise<void> => {
	const paths = [
		"Cargo.lock",
		"Cargo.toml",
		"app/Cargo.lock",
		"app/Cargo.toml",
		"fuzz/Cargo.lock",
		"app/notes.lock",
	];
	const client = await GitHub.create({
		owner: "P4suta",
		repo: "youarel",
		defaultBranch: "main",
		fetch: async (input: unknown): Promise<Response> => {
			assert.equal(typeof input, "string");
			if (typeof input !== "string")
				throw new Error("Unexpected request input.");
			const url = new URL(input);
			assert.equal(url.origin, "https://api.github.com");
			assert.equal(url.pathname, "/repos/P4suta/youarel/git/trees/main");
			const tree = paths.map(
				(path: string): Record<string, string> => ({
					path,
					mode: "100644",
					type: "blob",
					sha: "a".repeat(40),
				}),
			);
			return new Response(JSON.stringify({ tree, truncated: false }), {
				headers: { "content-type": "application/json" },
			});
		},
	});
	assert.deepEqual(await client.findFilesByGlob("**/Cargo.lock"), [
		"Cargo.lock",
		"app/Cargo.lock",
		"fuzz/Cargo.lock",
	]);
	assert.deepEqual(await client.findFilesByGlob("**/*.toml", "app"), [
		"Cargo.toml",
	]);
	assert.deepEqual(await client.findFilesByGlob("**/*.json"), []);
});

test("the locked CLI reads its token from a private file instead of a process argument", async (): Promise<void> => {
	const directory = await mkdtemp(join(tmpdir(), "youarel-proposal-"));
	try {
		const token = join(directory, "github-token");
		await writeFile(token, "fixture-token", { mode: 0o600 });
		assert.equal(coerceOption(token), "fixture-token");
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
});

interface ParsedFields {
	readonly [name: string]: unknown;
	readonly name?: unknown;
	readonly version?: unknown;
	readonly package?: unknown;
	readonly packages?: unknown;
	readonly workspace?: unknown;
	readonly path?: unknown;
	readonly jsonpath?: unknown;
	readonly type?: unknown;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function record(value: unknown): ParsedFields {
	assert.ok(isRecord(value));
	return value;
}

function array(value: unknown): readonly unknown[] {
	assert.ok(Array.isArray(value));
	return value;
}

function owned(name: string): boolean {
	return (
		(name.startsWith("youarel-") && name !== "youarel-fuzz") || name === "xtask"
	);
}

interface PackageVersion {
	readonly name: string;
	readonly version: string;
}

function packages(content: string): readonly PackageVersion[] {
	const values = array(record(parseToml(content)).package);
	const result: PackageVersion[] = [];
	for (const value of values) {
		const item = record(value);
		const name = item.name;
		const version = item.version;
		assert.equal(typeof name, "string");
		assert.equal(typeof version, "string");
		if (typeof name === "string" && typeof version === "string")
			result.push({ name, version });
	}
	return result;
}

test("the pinned release-please updaters synchronize owned versions without changing dependencies", async (): Promise<void> => {
	const source: unknown = JSON.parse(
		await readFile(resolve(ROOT, "release-please-config.json"), "utf8"),
	);
	const config = record(source);
	const product = record(record(config.packages)["."]);
	const extras = array(product["extra-files"]);
	assert.equal(extras.length, 6);
	for (const value of extras) {
		const extra = record(value);
		const path = extra.path;
		const jsonpath = extra.jsonpath;
		assert.equal(typeof path, "string");
		assert.equal(typeof jsonpath, "string");
		if (typeof path !== "string" || typeof jsonpath !== "string")
			throw new Error("An updater needs a path and JSONPath.");
		const before = await readFile(resolve(ROOT, path), "utf8");
		const after =
			extra.type === "json"
				? new GenericJson(jsonpath, TARGET_VERSION).updateContent(before)
				: new GenericToml(jsonpath, TARGET_VERSION).updateContent(before);
		assert.ok(after !== before, `${path} must change`);
		if (path.endsWith("Cargo.lock")) {
			const original = packages(before);
			const updated = packages(after);
			assert.equal(updated.length, original.length);
			let count = 0;
			for (const [place, { name, version }] of original.entries()) {
				assert.deepEqual(
					updated[place],
					{ name, version: owned(name) ? TARGET_VERSION.toString() : version },
					`${path}: ${name}`,
				);
				if (owned(name)) count += 1;
			}
			assert.ok(count > 0, `${path} must contain product packages`);
		} else if (path === "Cargo.toml") {
			assert.equal(
				record(record(record(parseToml(after)).workspace).package).version,
				TARGET_VERSION.toString(),
			);
		} else if (path === "app/Cargo.toml") {
			assert.equal(
				record(record(parseToml(after)).package).version,
				TARGET_VERSION.toString(),
			);
		} else {
			const parsed: unknown = JSON.parse(after);
			assert.equal(record(parsed).version, TARGET_VERSION.toString());
		}
	}
});
