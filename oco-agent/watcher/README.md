# The watcher outside the machine

It runs as `.github/workflows/watch-the-lab.yml` in this public repository: every quarter hour it checks
that oco-agent answered its last probe - a tiny job asked through the jobs repository by the driver
"watcher" - and alerts the urgent channel itself, once, when the lab has not answered for 45 minutes.

It first lived in the private jobs repository, probing lab-agent; there GitHub never ran its schedule
once (2026-11-12), so it moved here. Its secrets (LAB_TOKEN and the three NTFY_ values) are set with
`oco-agent/watcher/install`. Decisions: 2026-11-12-the-watcher-outside, 2026-11-12-the-watcher-moves.
