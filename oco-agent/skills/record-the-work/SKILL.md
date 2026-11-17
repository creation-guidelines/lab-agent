---
name: record-the-work
description: Where thinking, decisions, research, backlog, milestones and the author's words are recorded in the ecosystem, and how. Use whenever you decide, learn, start or finish something, or the author says something.
---

# Recording the work

Nothing may depend on anyone remembering: it goes into the owning project's own data, in the same change
as the work, and propagates to the observatory by itself.

**In linux-phone-oco, everything goes through tad in one database: linux-phone-oco/docs** (the author,
2026-11-17; docs' decision `2026-11-17-one-database`). Data through tad, landing through land, nothing of
ocoeco by hand: no `ocoeco note`, no glimpse, no oco/ tables in any linux-phone place. In a worktree of docs,
write with its named records, naming the place: `tad r decision --project surya-lab --id DATE-slug
--decision ... --why ... --rejected ...`; `tad r research`, `tad r source`, `tad r backlog`, `tad r
backlog-status`, `tad r session-log`, `tad r concept`, `tad r milestone-readiness`; read with `tad q backlog|
decisions|research [--project ...]`; `tad statements` lists them with their parameters. A missing one is a new
file in docs' `records/` or `queries/`, in the same change. `tad canon && tad render`, then `land`. The
commit and its pull request are the record of the work, signed by your OCOECO_BY. Why, measured: the turn
commit-title limit failed 4 times, link refusals ordered the work 3 times, two tools wrote rows in one place.

Everywhere else in the commons:

- **Decisions**: `ocoeco note decisions --id DATE-slug --set date=... --set decision=... --set why=...
  --set rejected=...` - always with what was rejected and why; `about=` a milestone where one fits.
- **Research**: `ocoeco note research --id ... --set question=... --set found=...` (sources inside the text
  unless the place has a sources table); in linux-phone-oco/docs, outside work is a `landscape` row
  (`tad set landscape ...`).
- **Backlog**: one row per piece of work; its note says where it stands and the next step; `status` done
  only when what landed is read back from main. Say on it that you take an item up (claim before work).
- **Session log**: `ocoeco note session_log --id DATE-slug --set entry_date=... --set title=... --set done=...`.
- **The author's words**: every turn of his, as said, with its moment - `turn ID AFTER AT TEXT SAID`
  (a word of oco-work) into marshall-oco/conversation.
- **Links**: ask `ocoeco fits` for an id; a link that points nowhere is refused. A milestone's criteria
  can only name rows that exist - enter those first.
- **Hand-work**: `ocoeco manual "ACTION"` in the place that keeps it; a third time by hand asks for a tool.
- **A gap or overlap in the ecosystem itself** is a sign: `ocoeco note problems` in ocoeco's place, answered
  by refining the whole.
