---
status: accepted
date: 2026-11-08
---
# Web address input defaults to HTTPS

## Context

The owner expects `google.com` to register without adding a scheme.
Saved links must remain admitted HTTP or HTTPS URLs.

## Decision

Complete recognizable host input with `https://` at user-input boundaries, preserving paths, queries, fragments, ports, and internationalized domains.
Use the shared link adapter and the [URL Standard](https://url.spec.whatwg.org/#url-parsing), then apply core admission.
Preserve explicit HTTP and HTTPS schemes.
Reject ordinary text, relative paths, email addresses, and unsupported explicit schemes.
Keep strict parsing for persisted links and opening.
The add sheet uses a text field with a URL keyboard hint because the [URL input type](https://html.spec.whatwg.org/multipage/input.html#url-state-(type=url)) represents absolute URLs.
Describe completion beside the field and show format suggestions under [W3C error guidance](https://www.w3.org/WAI/WCAG22/Understanding/error-suggestion.html).

## Alternatives

Requiring a scheme preserves strict input but adds repeated typing.
Frontend-only completion leaves clipboard, command-line, and direct command input inconsistent.

## Consequences

Services requiring HTTP need an explicit `http://` address.
Never downgrade after an HTTPS failure.
Reconsider single-label host completion if intranet use requires it.

## Confirmation

Check canonical storage, duplicate detection, title preview, clipboard input, drops, invalid input, and parser corpus replay.
