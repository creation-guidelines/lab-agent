---
name: land-a-change
description: How a change reaches main in the ecosystem - its own branch, checks, a record, land. Use whenever you are about to edit a repository under ~/Projects or land a change, including data-only changes.
---

# Landing a change

- **Branch first, then write**: `fresh OWNER/REPO BRANCH` resets the clone and cleans untracked files, so
  anything written before it is lost. Work only in places your brief says you own; another session may be
  working in the others.
- **Change what you mean**: `change FILE` with the exact old text, `--- becomes ---`, the new text; rows with
  `ocoeco note TABLE --id ID --set f=v` or, in a tad place (linux-phone-oco/docs), `tad set` / `tad sql`.
  Never copy a whole file from an older copy.
- **Check like CI**: Rust - `rust_checked` (fmt, clippy -D warnings, tests). linux-phone-oco/docs - after
  the last data change, `tad canon && tad render && make -s verify` (its CI compares the rendered pages; a
  row entered after rendering fails it). Shell scripts - `sh -n` and shellcheck.
- **Record in the same change**: a glimpse is made by `landit`; decisions (with what was rejected), the
  backlog item's next step and the session log go in the owning place's own data (record-the-work skill).
- **Land**: `landit "SUMMARY" KIND` (summary at most 88 characters; KIND feat, fix, docs, ...). It waits for
  every check and refuses on a failing one. If it stops, the branch is often already pushed: read
  `gh pr checks BRANCH` and the failing run's log (`gh run view ID --log-failed`), fix on that branch, push,
  then `land --no-manual`.
- **Read main back**: after landing, check that main holds the work (`git ls-files`, the rows), not only its
  record.
