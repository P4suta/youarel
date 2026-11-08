# Bookmark interface

The library keeps saving, finding, opening, organizing, and restoring bookmarks in one list.
Single pointer activation selects a bookmark or folder, and double-click opens its URL or location.
Names, icons, and row backgrounds use the same interaction.
Every row offers a context menu.
Folder rows open a location containing its direct folders and bookmarks.
Native button activation with Enter, Space, or assistive technology provides the keyboard equivalent under the [button pattern](https://www.w3.org/WAI/ARIA/apg/patterns/button/).
Empty folders open normally.
Folder icons and names distinguish containers without repeated type labels.
Inside a folder or Trash, the path follows the [breadcrumb pattern](https://www.w3.org/WAI/ARIA/apg/patterns/breadcrumb/) and contains ancestor actions and one current location.
The root omits a self-navigation button.
Long paths wrap under the Digital Agency's [breadcrumb guidance](https://design.digital.go.jp/dads/components/breadcrumb/usage/).
The interface omits the sidebar and fixed All, Unsorted, Recently added, and Collections navigation.
Trash appears as a system folder at the root and keeps its restoration and deletion actions.
Items in Trash show names and recovery actions without an opening control.
The name-focused list omits URL and date columns, while the edit sheet shows the full address and creation date.
The content choice follows [Apple's title-focused list guidance](https://developer.apple.com/design/human-interface-guidelines/lists-and-tables).
Every entry pairs its name with a recognizable icon in the same activation target.
Bookmarks keep a standard link icon while the site image is missing, loading, or invalid.
Generated letters and numbers never replace type icons.
Owner preference removes row checkboxes, separate disclosure buttons, and collection counts.
People select rows with a pointer, modifier keys, or keyboard commands.
Each name describes its selection state to assistive technology.
Bookmark descriptions identify their collection path for assistive technology.
Collection moves have a destination picker, so dragging is optional.
Folder contents receive initial keyboard focus without waiting for the native summon event.
The desktop uses [contextual commands](adr/window-commands-are-contextual.md) instead of permanent search, sort, creation, or row-menu buttons.
New bookmarks use the current folder without a destination picker.
Empty folders explain pointer and keyboard creation.

The design uses the [Web Content Accessibility Guidelines (WCAG) 2.2](https://www.w3.org/TR/WCAG22/) Level A and AA conditions as review criteria.
The Digital Agency's [button guidance](https://design.digital.go.jp/dads/components/button/usage/) and [input accessibility guidance](https://design.digital.go.jp/dads/components/input-text/accessibility/) guide labels, instructions, and error associations.
Targets for standalone actions are 48 pixels.
Contiguous rows keep the full target without extra vertical padding.
The measurement element uses the same content and styling as a visible name to prevent wrapping from inflating every row.
The interaction follows [rows selecting before opening](adr/rows-select-before-opening.md).
Hover inverts the row without changing selection.
Selection remains highlighted and adds a solid inset boundary after the pointer leaves.
Keyboard focus adds a dashed boundary, following [W3C focus visibility](https://www.w3.org/WAI/WCAG22/Understanding/focus-visible.html).
State boundaries keep contrast in both appearances and forced colors under [non-text contrast guidance](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html).
The desktop interface uses relative 16-pixel body text with a 1.5 line height and supports text enlargement, spacing overrides, and system contrast modes.
Forms use one reading column, consistent control dimensions, and instructions beside their fields.
Editing and moving choose one folder with a native select for up to eight folders, then a searchable combobox.
The threshold is a product choice under the Digital Agency's [guidance on small and large option sets](https://design.digital.go.jp/dads/components/combobox/).
The searchable field follows the [combobox keyboard and accessibility pattern](https://www.w3.org/WAI/ARIA/apg/patterns/combobox/), and accepts only an existing collection or no collection.
The body size exceeds Apple's macOS default and follows the Digital Agency's [typography baseline](https://design.digital.go.jp/dads/foundations/typography/).
The owner's 18-pixel preference applies to public websites.

The [Authoring Practices Guide dialog pattern](https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/) governs naming, focus containment, cancellation, and focus restoration.
Native dialog methods provide modality and cancellation under the [HTML dialog contract](https://html.spec.whatwg.org/multipage/interactive-elements.html#the-dialog-element).
The shared opener sets `aria-label` from the visible heading before showing a dialog.
The naming policy follows the workaround in [WebKit's report on dialog labels](https://bugs.webkit.org/show_bug.cgi?id=273846), and macOS checks verify the resulting name and fields.
The [menu pattern](https://www.w3.org/WAI/ARIA/apg/patterns/menubar/) governs action menu navigation.
Right-click, Shift+F10, and the Menu key open contextual actions.
Cancellation restores the invoking row or the current folder.
The macOS menu bar also offers the context commands under [Apple's context-menu guidance](https://developer.apple.com/design/human-interface-guidelines/context-menus).
The native menu prevents activation of unavailable commands, and context menus omit irrelevant commands.
Context-menu labels use familiar icons.
The list uses native list items with one individually reachable name per entry.
Virtual pages expose logical positions and the complete item count through [list item properties](https://www.w3.org/TR/wai-aria-1.2/#aria-setsize).
Virtual rendering controls the scroll position and disables browser anchoring through the [scroll anchoring draft's exclusion API](https://drafts.csswg.org/css-scroll-anchoring-1/#exclusion-api), preventing recycled rows from shifting keyboard focus out of the rendered range.
Selection updates preserve controls so a double-click survives the intervening redraw.
Name is a static heading.
Ordinary folders keep name order, and Trash uses its recovery listing.
The desktop rejects retired search and sort intents.
Form errors stay beside their fields or in a focused error summary, and failed submissions keep the draft.
Routine completion notifications disappear after five seconds without moving focus.
Their results remain available in the folder, editing history, or clipboard, applying the alternative-information condition in [W3C timing guidance](https://www.w3.org/WAI/WCAG22/Understanding/timing-adjustable.html).
Hover, keyboard focus, and a hidden window pause automatic dismissal.
Errors remain available until dismissed or replaced by another operation, and progress remains until completion.
New operations clear obsolete feedback, and background refreshes never revive an expired notification.
Status messages use polite announcements without taking focus under [W3C status-message guidance](https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html).
The Digital Agency's [notification guidance](https://design.digital.go.jp/dads/components/notification-banner/usage/) distinguishes completion, error, and warning purposes.
The desktop uses a compact status region for task feedback instead of a website announcement banner.

The Digital Agency discourages forms inside web modal dialogs.
The desktop library instead uses short, contextual sheets under [Apple's sheet guidance](https://developer.apple.com/design/human-interface-guidelines/sheets), with one sheet open at a time and a visible Cancel action.
The owner's black-and-white palette replaces Apple's general preference for system colors while retaining system forced colors and explicit focus, selection, and error indicators.
Media, authentication, payments, motion gestures, and timed sessions are absent, so their conditional requirements don't apply.
The installed Tauri app requires its embedded JavaScript runtime and has no independent no-script workflow.

`cargo xtask interface`, also run by `just check`, checks the actual renderer with pinned Playwright and axe-core dependencies.
The checks cover dialog semantics, keyboard workflows, failure recovery, target geometry, both appearances, 320-pixel reflow, 200-percent text, specified spacing overrides, and forced colors.
Palette fixtures verify rejection of color, gray, translucency, gradients, shadows, opacity, filters, and blending.
The Linux window test exercises real WebKit and the library boundary.
Native accessibility tree inspection and direct interaction complement browser checks.
Automated results don't establish WCAG conformance or screen-reader support across operating systems.
