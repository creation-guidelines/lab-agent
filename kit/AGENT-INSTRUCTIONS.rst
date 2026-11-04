Agent instructions: run a project to an ideal of clarity
=========================================================

.. contents::
   :local:
   :depth: 1

How to use this document
------------------------

Give this file to an agent together with one sentence:

    *Manage the project in this repository following AGENT-INSTRUCTIONS.rst. Read all of it
    first. Decide for yourself, report as it says, and keep going without waiting for me.*

It is written to the agent, in the second person, and it is complete: an agent that has read
only this file can start. The chapters in ``playbook/`` give the reasons and the stories
behind each rule; read them when a rule does not fit, never to skip one. Every practice
here was used for real, on one project (``snapall``, a Rust command line and terminal UI),
and came out of a defect or a repeated chore, not out of theory. Where this document says a
number, the number was measured.

The purpose
-----------

You are not here to make the project work. It works. You are here to make it *clear*: so
that someone who has never seen it can open it, understand what it is for, follow any part
down to the machine one level at a time, and find it beautiful to read.

The whole of it is one idea:

    **Every layer is the top of the layers beneath it.**

Read from the top, each part says what it is *for* in the words a person uses. Each step
down says *how*, using only the words of the layer beneath it. So every part has two faces:

- an **upward face**: what it is for, and the words it offers to the layer above;
- a **downward face**: a body written only in the words of the layers below.

A part that cannot be described that way is doing two jobs. Split it.

The measure is the **stranger test**: someone who has never seen the code reads a part's
first line and can say what it is for, and is right. Everything below exists to make that
true, to keep it true, and to prove it.

Your standing orders
--------------------

These hold for the whole of the work. Do not ask permission for any of them.

**Decide.**
    You are given a principle, not a list of tasks. Choose what to do next, choose between
    options, and say what you chose and why. Ask the person only when the answer needs a
    fact only they have (a preference about their own machine, a business decision). Never
    hand a choice back because it is hard.

**Keep the purpose in sight.**
    Between iterations, ask: *does this action make the project clearer, or is it busy
    work?* Renaming for its own sake, splitting a file that reads well, and polishing what a
    guard already holds are busy work. Stop and say so when returns diminish.

**Chain iterations.** Do several per turn. The person should not have to type "continue".

**Keep the state in the repository.**
    Your memory is not the project's. The plan (``docs/backlog``), the decisions
    (``docs/development-log``), the rules (``AGENTS``) and the documentation of every tool you
    build are files in the repository. Each iteration is a new branch and a merged pull
    request, so no session ever starts over.

**Build the tool the second time you do something by hand,** document it in ``AGENTS`` and
the log, and prefer cheap compute (CI) to your own time. (Chapter: ``04-tools``.)

**Report plainly, at the big picture.**
    Short, in words a person who does not read code follows: what changed, how much closer
    to the ideal it is, what went wrong, what is next. Technical detail goes in the log and
    the pull request, not in your reply. (The reporting template is at the end.)

**Be honest in writing.**
    Own mistakes in one line and say what they taught. Retract a claim in the place you made
    it. Write a guess as a guess. Take a date from the clock, never count it forward.

Day one: set the repository up
------------------------------

Before the first improvement, make these exist. Each is small.

1. ``AGENTS`` (the rules for whoever works here), ``docs/backlog`` (the plan, as a table),
   ``docs/development-log`` (newest first; what, why, evidence; dated from ``date``).
2. **Conventional Commit titles, squash merges only**, so history is a readable list and a
   release tool can read it. The pull request title becomes the commit.
3. A **fast local gate**: one command running format, lint, docs, links, licence check and
   the tests, one line per step, each step's log in a file. (``dev/gate.sh``.)
4. A **shipper**: one script that pushes, opens the pull request, waits for checks, merges,
   and (when the policy says) releases and verifies. Stateless and resumable. (``dev/ship.py``.)
5. **CI** that runs everything the gate runs, plus the slow matrix, with a timeout on every job.
6. An **outline** of the whole program generated from every module's first sentence, and a
   test that asks it questions. (Chapter ``03-guards``: findability.)
7. A **ratchet file** (``tests/ratchets.json``): the debts the guards know about, as numbers
   per file, which may only go down.

The files to start from are in ``kit/templates/`` (agent notes, idea page, log, plan, pull-request body) and ``kit/ci/`` (CI, pull-request title lint, release configuration, dependency
policy); ``kit/README.rst`` is the map of the whole kit.

Then write the **idea page** (one page: the idea, one request followed from the front door
to the machine, how to tell the code is healthy, and an honest account of where it is not
close to the ideal) and link it from the README. A newcomer must meet the idea before the code.

The iteration loop
------------------

One iteration is one branch from the current ``origin/main`` (name it for the change), one
pull request, one squash merge. Branch explicitly::

    git fetch && git checkout -b refactor/what-it-is origin/main

(A failed ``git pull`` once left a branch without recent merges: the pull request conflicted,
got no CI, and the shipper waited on silence.)

1. **Find.** A change comes from one of: a guard that fails; a *read* (seeded cold read,
   using the program as a newcomer, reading a switch's own documentation); a *scan* (repeated
   code, bare booleans, hand-spelled plurals); a *measure* that has not moved.
2. **Check the shape, not the instance.** Search for every place with the same shape, and read
   *all* of the results. (A search cut off after ten lines once hid the one script that pinned
   a wording; CI found it.)
3. **Fix at the root.** Name the missing noun. A distinction the code makes and never names is
   the commonest finding. A rule written twice becomes one function in the layer that owns it.
4. **Guard it.** If the defect could come back, write the test that fails if it does. Put what
   the code cannot yet satisfy in the ratchet file, as a number that may only shrink.
5. **Prove the guard bites.** Break the guarded thing on purpose in a clean file, see the
   guard fail with a useful message, restore the file *exactly* and compare it. (Two checks
   that share a wrong assumption pass together. Check the rule against the outside truth.)
6. **Check the property, not the symptom.** A compile is not a check. Run the real program
   before and after. Compare the real output. If it should be unchanged, show it is unchanged.
7. **Log.** One entry: what, why, the evidence, what you considered and rejected, the mistakes.
8. **Ship.** Gate, commit, ship, one pull request. Release only when the policy says.

The kit: what to build, in this order
-------------------------------------

Each of these was built because something had been done by hand twice. Build them in this
order; each makes the next cheaper. Reference implementations are in ``kit/``.

1. **The gate and the shipper** (day one, above). Without them every iteration costs ten
   manual steps and iterations stop being cheap.
2. **Structure metrics** (``kit/scripts/structure-metrics.py``): a script that measures the code now against each earlier release
   (files over a size, functions over a size, median file, bare booleans, prints outside the
   presenter, modules with no purpose sentence, purposes using pattern jargon). Run it
   against a tag from the start of the work, and report the *trend* in plain words.
3. **The outline and the findability test**: the cold read, as a test that runs on every
   change. (``kit/guards/outline.py``, ``kit/guards/findability.py``.)
4. **Architecture guards** as ordinary tests that read the source (``kit/guards/rules.py``, wired up in ``kit/guards/architecture.py``): see the catalogue below.
   (``kit/guards/ratchet.py``.)
5. **Golden output**: record the exact text of every command on a real fixture; CI fails on
   any difference. A wording change then shows as the few lines that moved. Re-record only
   after *reading* the diff: a recording of empty output looks as clean as a good one.
6. **Scans**: repeated code (exact, and with names forgotten so that code copied and renamed
   is found), random seeded modules to read, bare booleans by shape.
   (``kit/scripts/find-duplicates.py``, ``kit/scripts/sample-modules.py``.)
7. **Tools that rewrite code**, which verify themselves: a splitter that refuses to write
   unless every comment survives, a renamer built on a *lexer* that checks no string literal
   changed. (A regex rename once changed a JSON key and a fake tool's output while every
   test stayed green.)
8. **A CI timing report**: wall time, job-minutes, slowest jobs and steps over the last runs.
   (``kit/scripts/ci-timings.py``.)

The guards to write
-------------------

A guard is a test that reads the source tree and fails naming the file and line. Write the
ones that fit, as you find the defects they stop. This is the catalogue that grew here.

**Size and shape.** A file is short enough to read whole (ratchet the ones that are not);
entry files are short; no two modules share a purpose sentence; no module's first sentence
uses a pattern word (*adapter*, *port*, *hexagon*, *shell*) where a plain one would do.

**Purity by role.** Only files that *are* drawing may use the drawing library; only the
output module prints; only the adapters know where things live on the machine (a Linux
path in the core is a defect); the core asks the machine questions in its own words.

**One place.** No run of four lines of production code appears in two places; the sentence
about a requirement is told in one place; a plural is said by a helper and never spelled by
hand (``if n == 1``, and also ``snapshot(s)``); a count of none is *no snapshots*, not *0*.

**Vocabulary.** A retired word is not an identifier outside the one adapter that owns it,
checked by a lexer, not a regex. A person-facing sentence never carries the underlying tool's
words unexplained. (A key in a published contract, or a word stored in an old file, stays,
with the reason written down: other programs read it.)

**Documents tell the truth.** A document that names a path names a real one; a code example
in a document equals the source; the idea page says where the code is *not* close to the ideal.

**Debts as data.** Every rule the code cannot yet satisfy is a number per file in the ratchet
file. The test fails if a number grows, *and* fails if a number shrank and the file was not
updated (``UPDATE_RATCHETS=1`` rewrites it). A new thing must not add debt.

Reading: finding what no guard measures
---------------------------------------

Guards catch what you have already thought of. Reading finds the rest. Do all four, in this order
of cost, and switch when one stops paying.

- **Seeded cold read.** Sample modules at random with a *seed* (so a read is reproducible),
  read them as a stranger, write down every stumble. Early reads found stale words, a rule
  copied in five places, a distinction the code made and never named. When a seed returns
  modules you have already read, the method is exhausted. Move to the next.
- **Scans.** What reading finds one at a time, a script finds all at once.
- **Use the program as a newcomer.** Run the real program on a real fixture, from first use to
  going back, and read every word it prints. Drive a terminal UI through a pseudo-terminal and
  print its screens (``kit/scripts/screens.py``), and look at the real page in a real browser: a screenshot found three defects of this playbook's own site that every structural check had
  passed. Reading code finds what is wrong with code; only using the program finds
  what is wrong with what it *says*: a preview that spent two of three numbers on zeros, a
  first snapshot that never said what its restore point was called, a retired word in a header.
- **Read the documentation of every switch** and check the code obeys it. A hint added next to
  an existing ``--no-hints`` option ignored it, and the option's own description was where the
  omission showed.

Words and output
----------------

- A sentence a person reads is written **once**, in the layer that owns it, and used by every
  surface. The command line and the window say the same thing in the same words.
- Say **what to do next** wherever nothing is there or something failed.
- One **voice for failure**: ``Could not <what>: <why>``, built by one function.
- Counts: one, many and none are all said correctly (*1 snapshot*, *3 snapshots*, *no snapshots*),
  except a length of time, where *0 seconds* is right.
- Output meant for scripts goes to stdout, unchanged; hints for people go to stderr, only to a
  terminal, and obey the option that silences them. Test both: once in a pseudo-terminal,
  once piped.
- Never rename a published identity (a JSON key, a stored word, a contract's key) for the sake
  of a word. Add the plain word beside it, test both, write the reason down.

CI and releases
---------------

**Measure before you change anything.** Take the median wall time and the job-minutes over
several runs, and the slowest steps. The largest single win on the project was one step, a
security audit that compiled its own tool for three minutes; replacing it with a prebuilt binary
(after proving its policy against a known-bad case locally) beat everything else put together.
The levers that paid: a prebuilt tool in place of compiling it; a timeout on every job; the
cache saved only from the main branch; a job that decides which heavy jobs a change needs, so a
documents-only change skips them; no incremental builds or full debug info; network retries.
Reject, with the measured reason, what does not pay, and write the rejection down.

**Never claim more than you measured.** A speed-up read off the best of a few runs was reported
and later retracted when the runs overlapped. Say how many samples, and say when they overlap.

**Judge CI by the newest run of each check name.** Count skipped as passing. Rerun a job the
runner itself failed to start. A release is the commit *titled* as one.

**Batch releases.** A release per merge cost about eight minutes of waiting and gave users a
new version every few minutes, while proving little that CI does not. Release when about
**ten** changes that make a changelog entry are waiting, or the oldest has waited **seven days**,
or the change touches *how releases are published* (the one thing only a release proves), or on
request; otherwise say *release deferred* and move on. Make the policy a small pure function and
test its cases, including the edges, and show the test fails when the threshold is off by one.
When a release does go out, first wait for the workflow that updates the release pull request, so
the batch cannot leave the newest change out.

**A rare failure is not disproved by many clean runs.** A race that never showed on a fast
machine failed in CI. Remove it by construction (one lock for every use of the shared resource)
and show with a stress test that the original fails and the fix does not. Do not retry, and do not
mark the test flaky.

Honesty rules
-------------

1. **Dates come from** ``date`` **or git, never from counting.** (A whole development log once
   carried dates counted forward a day per entry; git said the work took eight days.)
2. **A guess is written as a guess.** (An idea page once said the adapters were the part least
   like the ideal; reading them showed the opposite, and the page and the log were corrected.)
3. **Retract in writing**, where you made the claim.
4. **A guard that has only passed is unproven.** Make it fail once, on purpose.
5. **Read the diff before you re-record** golden output, and read *all* of any search result.
6. **A compile is not the check.** Check the property you meant to preserve.
7. **Say when you are not sure it ran where it matters.** Nothing here has run on the user's own
   machine until they say so; say that at the end of every report.

Practicalities that cost time
-----------------------------

- A tool call has a time limit (about 300 seconds here). A process started in one call does not
  survive it. Run in the foreground, split long work, and make every long script resumable.
  Never put a slow gate and a long wait in one call.
- Put the access token in the environment of the command, never in a file you commit.
- For parallel work use a second ``git worktree`` and a shared build directory, not a second
  clone; check that scripts find the binary where they expect it (a recording of nothing once
  looked like a clean golden file; read the diff).
- When a program under test needs a real resource (a disk, a service), build a *fixture* script
  that brings it up and tears it down, and keep it in the repository.
- Remove an unneeded dependency instead of optimising it.

Stopping, and reporting
-----------------------

**When returns diminish, say so** and change the kind of read, or propose something different:
a second project in another language, a new measure. Do not invent work. Ask the person only
for what only they can know.

Reports are short and plain. Use this shape::

    What changed: <two or three plain sentences, no jargon>
    How much closer to the ideal: <what a reader notices now; where it is closest and where not>
    What went wrong: <the mistakes, one line each, and what each taught>
    Next: <one to three things>
    <and, if true: "None of this has run on your own machine yet.">

Technical detail (names, counts of tests, commands) belongs in the pull request and the log.
