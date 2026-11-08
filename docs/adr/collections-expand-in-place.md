---
status: superseded
date: 2026-11-08
---
# Collections expand in place

Superseded by [folders showing their contents](folders-show-their-contents.md).

## Context

Collection names navigate or expand, and the list and sidebar hold separate expansion states.
The owner wants one expansion operation and visible tree relationships.
Generated site initials can resemble bookmark counts.

## Decision

Share one expansion state between the list and sidebar.
Collection names toggle descendants in the bookmark tree.
The sidebar reveals the same tree instead of opening an isolated collection.
Render branch connectors from complete listing facts so virtual scrolling preserves their meaning.
Label collections explicitly and omit generated icon substitutes.
Keep native buttons and table semantics under the [disclosure pattern](https://www.w3.org/WAI/ARIA/apg/patterns/disclosure/) and [table pattern](https://www.w3.org/WAI/ARIA/apg/patterns/table/).

## Alternatives

Independent folder navigation supports focused views but preserves the unwanted second operation.
Inferring connectors from visible rows avoids a view field but loses offscreen siblings.

## Consequences

Expanded descendants share one paged listing.
Empty collections expose their name and actions without an ineffective expansion button.
Reconsider the navigation policy if the owner requests isolated collection views.

## Confirmation

Pointer and keyboard activation must toggle the same branch.
Sidebar activation must reveal that branch in the shared tree.
Connectors must distinguish nested and final siblings across page boundaries.
