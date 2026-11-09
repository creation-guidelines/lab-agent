# Changelog

All notable changes to lab-agent are documented here.
## [0.1.15]


### Bug Fixes
- Board wait/pause/resume/done changed nothing ([#42](https://github.com/creation-guidelines/lab-agent/pull/42)) ([6b85e85])



## [0.1.14]


### Features
- The board as a live TUI, with task states ([#40](https://github.com/creation-guidelines/lab-agent/pull/40)) ([3bd4366])



## [0.1.13]


### Features
- The board and the inbox - follow a long session from the phone, and speak into it ([#37](https://github.com/creation-guidelines/lab-agent/pull/37)) ([6de0f8c])



## [0.1.12]


### Bug Fixes
- Notify-setup gives the phone links it can open, as QR codes, and the right DND advice ([#35](https://github.com/creation-guidelines/lab-agent/pull/35)) ([3ee81b4])



## [0.1.11]


### Features
- Phone notifications through ntfy - private channels per user, and jobs that tell you how they ended ([#33](https://github.com/creation-guidelines/lab-agent/pull/33)) ([1c96ede])



## [0.1.10]


### Features
- The monitor spends no GitHub budget - conditional reads, and local records on the machine that runs the agent ([#31](https://github.com/creation-guidelines/lab-agent/pull/31)) ([e64ab69])



## [0.1.9]


### Features
- A result is never lost to a failed push, and a stale 'running' row says so ([#29](https://github.com/creation-guidelines/lab-agent/pull/29)) ([1fde085])



## [0.1.8]


### Bug Fixes
- Idle polling costs nothing, and a spent GitHub budget is waited out instead of losing a result ([#28](https://github.com/creation-guidelines/lab-agent/pull/28)) ([4642e9f])



## [0.1.7]


### Miscellaneous
- A pinned, flake-free nix-shell development environment ([#24](https://github.com/creation-guidelines/lab-agent/pull/24)) ([3f62ea2])



## [0.1.6]


### Bug Fixes
- Keep the rpm assets in their own table - the requires sub-table had split them off ([#22](https://github.com/creation-guidelines/lab-agent/pull/22)) ([4e40ccb])



## [0.1.5]


### Features
- The agent API - submit, wait, logs; env-configured client; partial logs ([#21](https://github.com/creation-guidelines/lab-agent/pull/21)) ([e44dd2e])



## [0.1.4]


### Features
- One argenv contract; completions and man page generated from it; jobs, self-update, live monitor ([#17](https://github.com/creation-guidelines/lab-agent/pull/17)) ([8a93419])



## [0.1.3]


### Bug Fixes
- Connect refuses to run as root, and no longer lies about systemd setup succeeding ([#15](https://github.com/creation-guidelines/lab-agent/pull/15)) ([894b72a])



## [0.1.2]


### Bug Fixes
- Disable LTO for the Arch package - ring's C/asm FFI boundary breaks under it ([#13](https://github.com/creation-guidelines/lab-agent/pull/13)) ([a1b8bef])



## [0.1.1]


### Bug Fixes
- Stop pages.yml and repos.yml racing on the same GitHub Pages deployment ([#10](https://github.com/creation-guidelines/lab-agent/pull/10)) ([4851ac7])
- The real release tag is v<version>, not lab-agent-v<version> ([#11](https://github.com/creation-guidelines/lab-agent/pull/11)) ([58e48e4])
- Install the musl target's std lib before cross-compiling ([#12](https://github.com/creation-guidelines/lab-agent/pull/12)) ([86e8832])


### Miscellaneous
- Release v0.1.0 ([#8](https://github.com/creation-guidelines/lab-agent/pull/8)) ([d58b088])



## [0.1.0]


### Bug Fixes
- README does not imply a system pip ([#4](https://github.com/creation-guidelines/lab-agent/pull/4)) ([b8e4642])
- Home no longer defaults to a root-only system path ([#5](https://github.com/creation-guidelines/lab-agent/pull/5)) ([fe84fc1])
- StartLimitIntervalSec section, and the author-date fix finished ([#6](https://github.com/creation-guidelines/lab-agent/pull/6)) ([0a40d54])


### Features
- The pull-based lab agent ([#1](https://github.com/creation-guidelines/lab-agent/pull/1)) ([e948843])
- Connect.sh, and drop SSH entirely in favour of a token ([#3](https://github.com/creation-guidelines/lab-agent/pull/3)) ([5b0f016])
- Rewrite lab-agent in Rust - parallel jobs, group-kill timeouts, root-exec, monitor TUI ([#7](https://github.com/creation-guidelines/lab-agent/pull/7)) ([783f884])


### Miscellaneous
- .gitignore, and remove a build artifact ([#2](https://github.com/creation-guidelines/lab-agent/pull/2)) ([dea43d4])


