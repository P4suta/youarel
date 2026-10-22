---
status: proposed
date: 2026-10-20
---
# Search ranks a bounded candidate set

## Context

The window and the terminal launcher searched by reading every shortcut of a scope.
Each shortcut's title, link, alias, folder path, and note took a subsequence match, so `gh` found `GitHub` and `ghost` alike.
The matches ranked by `match_rules::score` and then by `rank_rules::frecency`.
A search runs on each keystroke.
At ten million shortcuts, the read of every record alone would take seconds per keystroke.
[The record of the indexed store](the-state-is-kept-in-an-indexed-store.md) files each word of a shortcut's texts in `by_token` for this change.
It folds each word and cuts it to 12 bytes.

## Decision

`search_rules::strategy` decides how a search finds what it ranks.
It reads the number of live shortcuts in the search's scope and the words of its query.
A scope of fewer than `SCAN_LIMIT`, 50,000 shortcuts, keeps today's search and reads and scores every shortcut.
A larger scope reads only the shortcuts with a word that starts with the longest word typed.
When two words tie for the longest, the search takes the first of them.
The store first reads the `by_token` rows of words equal to that word, and then the rows of longer words that start with it.
It stops at `CANDIDATE_LIMIT`, 2,000 shortcuts.
It ranks those candidates as before, by `match_rules::score` over the whole query and then by frecency.
A query without a word, such as one of punctuation alone, finds nothing in a larger scope.
`index_rules::query_words` cuts a query's words as `index_rules::tokens` cuts the words it files.
A typed word and a filed word thus agree on folding and length.

Past 50,000 shortcuts, a word must start with what a person typed.
`gh` then finds a title with the word `ghost`, yet no title that holds `GitHub` only as `g` and `h` apart.
The scan limit counts the scope, not the library.
A folder of a few thousand shortcuts keeps the loose match inside a library of millions.

The same bound limits what a listing gathers in memory.
A grouping by site gathers its shortcuts in memory while the listing holds at most `ON_DEMAND_LIMIT`, 50,000 of them.
A larger listing shows them without headings, and the window says so.
Some shortcuts' kept starts of 32 bytes tie, and a run of them sorts by the whole texts of their records.
A run of more than 50,000 keeps the order of its keys instead.

Kani proves that a plan scans exactly below the limit, and that the longest word comes first among those no word passes.
Kani also proves that the bounds of a word hold exactly the keys that start with it.

Rejected alternatives:

- A scan of every shortcut on each keystroke, because its cost grows with the library rather than with the matches.
- An index of every substring or of trigrams, because it would multiply the index of words.
  That index already takes about a quarter of the database.
- A search over the ranges of every typed word, intersected.
  The longest word alone narrows the candidates most for the least reading, and the score ranks the rest of the query.
- A fixed size of library for the loose match.
  A person who searches a small folder expects the same results whatever the rest of the library holds.

## Consequences

A search in a large scope reads at most 2,000 records and the index rows of one word, whatever the library holds.
A shortcut whose only match lies past the first 2,000 candidates of a common word goes unfound until a longer word narrows them.
A person who types the start of a word finds what they look for in a scope of any size.
The terminal launcher and `youarel find` share the strategy, since both search through the store's session.
