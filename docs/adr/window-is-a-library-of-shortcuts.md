---
status: amended
date: 2026-10-18
supersedes: window-launcher-renders-the-terminal-launchers-machine.md
amended-by: the-window-draws-a-page-of-the-library.md, the-window-behaves-like-the-finder.md
---
# The window is a library of shortcuts

## Context

[The window launcher renders the terminal launcher's machine](window-launcher-renders-the-terminal-launchers-machine.md) made the window a popup that draws one list of `ui_rules::step` and answers only to the keyboard.
The owner wants a file explorer for links more than a launcher: a person saves the many pages of a project, such as dozens of reference pages of one site, into folders, closes the tabs, and comes back to them by walking the folders.
The owner's design shows a sidebar of collections with their counts, cards that set each link as type, a sortable list, columns that walk the folders beside a preview, sheets to add and edit, and settings.
A pointer and drag and drop drive much of that design, and a key machine of one list cannot hold three layouts, a selection of many shortcuts, or a folder walk.
The terminal launcher still needs its machine, its Quint model, and its replay, unchanged.

## Decision

The window becomes an ordinary app window, the library, which a global shortcut, the tray, a second start, or the Dock brings forward with the search focused.
It never hides when it loses the focus or after it opens a link, and closing it hides it while the tray keeps it running.
It opens in the layout a person last chose, the columns at first, as a file explorer's column view does.
It behaves as an app rather than a page: on macOS its menu bar holds the settings, the new link and collection, the import, and the layouts.
A right click shows the window's own menus and never the webview's, and the select all command selects the folders and shortcuts shown.
A bookmark file dropped on the window imports, and a picture never drags out on its own.

Folders, which the window calls collections, are the one way to group shortcuts, and they nest as deep as a person likes.
A shortcut lives in one folder or at the top, which the window lists as unsorted.
A folder holds a link at most once, and another folder may hold the same link, as two folders may hold copies of one file.
The window works where a person stands, as a file explorer does: a new folder or a new link goes into the deepest folder the columns open, and the trail above the columns leads back up.
The cards and the list stand in one folder at a time, the top for all, and show its folders first and then its own shortcuts.
A double click or the return key opens a folder there, and the trail from the top leads back up, as does the command key with the up arrow.
A search, the unsorted shortcuts, and the recent ones list flat across the folders, each shortcut with the path of its folder.
The columns keep the deepest column at the right edge as the walk goes deeper or turns back up.
A shortcut without an icon shows its site's first letter on a color its site always takes, so a row of such shortcuts still tells apart.
The list also opens a folder in place, by its triangle, a double press, or the return key, and the sidebar or the trail steps into it.
The sidebar shows the collections as a tree that opens to the folder the window stands in.
A heading of the list sorts by its column, and a person may hide the address, the collection, or the date added.
A folder may carry a mark, such as one emoji, which every place shows in place of its glyph.
A picker sets it: the emoji under their categories, a search by the names that shortcodes write, and a preview, and a typed `:books:` writes 📚.
The window keeps its own short list of emoji and their shortcode names rather than a whole dataset, since a folder needs a mark it can find, not every emoji.
The mark travels inside the sealed state like the folder's name, and `mark_rules` admits it with a Kani harness.
The add sheet takes one link, and a link its folder holds already selects that shortcut instead of reading the page again.
Folders move by dragging them onto another folder, and a double click on a name, a title, an alias, or a note types over it in place.
The search covers titles, links, aliases, folder paths, and notes, so a shortcut needs no other label to be found.
Every layout may gather its shortcuts under headings, as a file explorer groups its files, by facts the links and the library already hold: the site, the collection, or the age.
A site is a link's registrable domain, as the web's rule of the same site draws it, so the pages of one site group together across its subdomains.
An age falls in the past seven days, the past thirty days, or earlier, counted back in whole spans of days so that no time zone moves a shortcut between headings.
Each column shows its folders first and its shortcuts under their headings, and a heading begins a row of the cards, so up and down move a visible row across the headings.
`library_rules` decides the groups, the periods, and the moves across the headings, with Kani harnesses.

Rust still holds every decision, and the page stays a renderer.
The page sends intents, such as choosing a scope, selecting a shortcut, a key that moves the selection, or a drop on a folder, and `app/src/library.rs` answers each with the whole view.
`library_rules` decides what a scope lists and in which order, the counts of every scope, and where a key moves the selection in the grid, the list, and the columns, with Kani harnesses beside each decision.
`host_rules` decides how a link's host splits into the subdomains, the registrable name, and the suffix that the cards set as type.
The window's state per intent is plain assignment, so no second step machine and no second Quint model arise.

The page is strict TypeScript in a few ES modules that `tsc` compiles beside their sources, with no bundler and no framework.
It sets every text as text, under the same content security policy, which admits only its own files, `data:` images, and its IPC.
Icons cross once by name, drawn again by youarel's decoder, so a view stays small as the library grows.
A capability file grants the page only the events that summon the window and the dragging of the window by its toolbar.
The clipboard plugin reads the links on the clipboard into the add sheet and copies links, and a file field reads a bookmark file in the page.
The window's preferences live in `settings.json` beside the store, outside the sealed state, because they belong to one device.

Rejected alternatives:

- Tags beside folders, because research on personal information management finds that people keep their own things in folders and walk them, while tags see little use and their vocabulary drifts, and a second way to group asks a person to file each link twice.
- Grouping by the whole host, because `developer.mozilla.org` and `www.mozilla.org` would then split one site.
- Grouping by calendar days, because a day's boundary depends on the time zone, which the decision would then have to receive.
- A cap on the depth of folders, because an explorer's folders nest freely, and a cap would flatten the trees that browsers' bookmarks already hold.
- A link held once across the whole library, because one reference page belongs to several projects, as a file may be copied into several folders.
- Keeping the popup and adding pointer handling to `ui_rules`, because one key machine for the terminal and three pointer layouts would grow a state space that its Quint model could not cover and the terminal would never use.
- A step machine for the library with its own Quint model, because the window's state changes are assignments, and the decisions worth proving are the listing, the counts, and the moves, which pure functions hold.
- A front-end framework, because a renderer of one view still needs none, and its dependency tree would join the release.

## Consequences

The terminal launcher keeps `ui_rules`, and the window no longer shares its keys; the window's keys follow the app conventions of each desktop instead.
`youarel add` refuses a link its folder holds already, as the window skips it.
Every intent reloads the store and answers with the whole view, which a library of a few thousand shortcuts affords.
The window test in `just app-linux` adds a link through the add sheet and hides the window with a second toggle, rather than naming a folder with the launcher's keys.
