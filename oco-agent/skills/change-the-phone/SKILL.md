---
name: change-the-phone
description: How to work with a lab phone safely - only through devlab, one change at a time, tests first, every change ending in the phone's own health report. Use before any action that changes a phone's state (reboot, kernel, flash, packages, config) and when adding a test.
---

# Changing the phone

- **Only through devlab** (`linux-phone-oco/lab-tools`, reading the lab's `device.toml`): `devlab phone sh|root`
  with commands on standard input, `phone put`, `phone reboot`, `kernel install|try|remove|log`,
  `suite put|run|latest`, `gather now --land`, or the same through the MCP door `devlab mcp`. Never a
  hand-written ssh: devlab holds the phone for every change (one at a time, across every session) and a
  second change waits. Never wrap devlab in a lock of your own - it would wait on itself.
- **Keep it known-working**: a test kernel is a second boot entry tried once by boot counting, falling back
  to stock by itself; never the initramfs debug shell on an unattended try. A reboot, a try and a provision
  end with the new boot's health report - read it; a command fails when the phone is unhealthy.
- **Test first**: a check that fails for the missing part, before the fix. Generic: lab-tools
  `health/suite/checks/{base,shell}/` (and its line in `devlab/src/suite.rs`); the phone's own:
  `<lab>/tests/health/{base,shell}/`. A check prints `pass|fail|skip TEST COMPONENT detail`. Enter what its
  pass proves in linux-phone-oco/docs' `tests` table (`present` or `works`): only a passing `works` test
  turns a part `works` on the face.
- **Results file themselves**: the phone runs its suite 120 s after every boot; `devlab watch` (a user
  service on the lab) brings each report, files it through tad and lands it; the face redraws. Never type a
  status a machine can measure; a person's witness goes in support_status `judged` and `notes`.
- **Witnessed tests**: before anything the author must hear or see, say what to expect and what to check,
  and wait for his go. Notify urgently only when his hands or eyes are needed.
