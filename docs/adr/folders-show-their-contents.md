---
status: accepted
date: 2026-11-08
---
# Folders show their contents

## Context

The owner chose folder navigation and a name-focused library without a sidebar.
The choice replaces [collections expanding in place](collections-expand-in-place.md).

## Decision

Show the direct contents of one folder at a time.
Keep the current location and ancestors preceding the list under the [W3C breadcrumb pattern](https://www.w3.org/WAI/ARIA/apg/patterns/breadcrumb/).
Wrap long paths following the Digital Agency's [breadcrumb guidance](https://design.digital.go.jp/dads/components/breadcrumb/usage/).
Use Folder in visible controls and recognizable type icons beside names.
Bookmarks use a cached site image or a standard link icon.
Present Trash as a system folder at the root.
Omit sidebar scopes, repeated type labels, URL columns, and date columns.
Keep complete bookmark details in the edit sheet, following [Apple's title-focused list guidance](https://developer.apple.com/design/human-interface-guidelines/lists-and-tables).

## Alternatives

Inline expansion shows parent and child contents together but keeps hierarchy controls the owner no longer wants.
Permanent metadata columns help compare addresses and dates but compete with bookmark names.

## Consequences

Opening another branch changes location.
Search and Show in Folder must keep access to nested bookmarks.
Reconsider parallel branches if organizing bookmarks requires simultaneous locations.

## Confirmation

Whole-row double-click and keyboard activation open folders, including empty folders.
Breadcrumb ancestors restore their direct contents.
Narrow layouts, virtual scrolling, and Trash recovery remain operable.
