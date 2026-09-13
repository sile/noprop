# backlog

This directory holds design proposals (RFCs) and, in the future, other
unprocessed work items such as TODOs and bug reports for noprop.

## Purpose

An RFC here records *how a decision was reached*, not *what the current
specification is*.

The authoritative specification is:

- the rustdoc comments in `src/` for the public API, and
- the documents under `docs/` for concepts and recipes.

RFCs exist so that future maintainers can reconstruct the reasoning behind a
change. They are not a spec and must not be treated as one. Code and `docs/`
must stand on their own and must **not** reference a file in `backlog/`; if a
decision matters to a reader, write it where the reader is looking instead.

## Layout

```
backlog/
  README.md                       # this file: process, naming, states
  template-rfc.md                 # template for a new RFC
  YYYYMMDD-rfc-slug.md            # an open RFC
  done/
    YYYYMMDD-rfc-slug.md          # a settled RFC
```

Only two directories exist: `backlog/` for open items and `backlog/done/` for
settled ones. Item *kind* is carried by a filename prefix (`rfc-` today;
`todo-` and `bug-` if such items are added later), not by a directory. Keeping
the tree shallow is intentional: a file is always one or two levels deep.

## Naming

`YYYYMMDD-<kind>-<slug>.md`

- `YYYYMMDD` is the date the item was created, e.g. `20260913`.
- `<kind>` is `rfc` today.
- `<slug>` is a short lower-case hyphenated summary, e.g.
  `add-typed-samplers`.

Example: `20260913-rfc-add-typed-samplers.md`

There is intentionally **no sequence number**. RFCs are not cited by number,
and neither `src/` nor `docs/` may reference them, so a stable identifier buys
nothing. The date prefix sorts the directory chronologically, which makes it
obvious at a glance which proposals are oldest. Collisions are avoided by the
slug; two proposals on the same day simply get different slugs. If two
proposals would share a date and a slug, they are really one proposal and
should be merged.

Because no numbering is used, there is no counter file to maintain.

## States

An RFC has one of two states, expressed by its location:

| State  | Location          | Meaning                                        |
| ------ | ----------------- | ---------------------------------------------- |
| `open` | `backlog/`        | Being drafted or discussed.                    |
| `done` | `backlog/done/`   | Settled; no longer being worked on.            |

Whether a settled RFC was **accepted** or **rejected** is recorded in its
`Status` field (see `template-rfc.md`) and in its body. It is deliberately not
split into separate directories:

- noprop does not value long-term stability as a primary goal, so "accepted"
is not a permanent commitment. An accepted RFC may later be changed or even
removed, which makes an "accepted" label on an old RFC misleading rather than
informative.
- `accepted` and `rejected` are the same operational state: the discussion is
over and nobody is working on it. One `done/` directory expresses that; the
qualitative difference belongs in the text, where the reasoning lives.

`postponed` proposals stay `open` (in `backlog/`), because they are still
unresolved.

## Moving an item to `done/`

Use `git mv` so history follows the file:

```sh
git mv backlog/20260913-rfc-add-typed-samplers.md backlog/done/
```

Update the `Status` field in the same commit.

## Packaging

The `backlog/` directory is development material and is excluded from the
published crate. This is configured in `Cargo.toml`:

```toml
exclude = ["backlog/"]
```

Do not remove that line. It keeps proposals out of the crate tarball without
affecting what git tracks.
