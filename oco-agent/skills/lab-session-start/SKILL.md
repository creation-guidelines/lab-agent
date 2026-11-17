---
name: lab-session-start
description: Start of every session on the lab machine (open-commons ecosystem, linux-phone-oco and the rest) - read the live practices, sign your work, read the inbox and the lenses before choosing anything. Use at the very beginning of a session, and again after a long pause.
---

# Starting a session on the lab

1. `lab-agent guide` - the live practices. Read them whole; they outrank any brief.
2. In every shell that works on the ecosystem: `export OCOECO_BY=<your role>` (bring-up, orchestration,
   research-<feature>, ...) then `. oco-work`. Your name goes on every record and signs every commit (oco-work makes it the git author), so the face shows where each
   session stands. Never call `land` from a shell that has not sourced `oco-work` (without it `land` also
   merges pending release PRs).
3. `lab-agent inbox` - print every unread message, answer each, then `lab-agent inbox --ack`.
4. Look before choosing: from `~/Projects/marshall-oco/marshall-oco.github.io`, `ocoeco focus still-showing`;
   the board (`lab-agent board show`); for linux-phone, the face (https://linux-phone-oco.github.io/docs/)
   and `tad sql` in `~/Projects/linux-phone-oco/docs`. Your brief is in a project's `prompts/`
   (linux-phone-oco/docs: bring-up, orchestration, feature-research).
5. Choose by the weakest link the data shows, not by momentum; say on the board what you take up.
