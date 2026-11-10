# Changelog

All notable changes to lab-agent are documented here.
## [0.1.30]


### Features
- Self-update --within SECONDS waits for a newer lab-agent ([#78](https://github.com/creation-guidelines/lab-agent/pull/78)) ([ba358c1])



## [0.1.29]


### Features
- The board shows which agent runs - and an installed update waiting for idle ([#76](https://github.com/creation-guidelines/lab-agent/pull/76)) ([8e24ed2])



## [0.1.28]


### Features
- Updates apply themselves when the agent is idle ([#74](https://github.com/creation-guidelines/lab-agent/pull/74)) ([c8142eb])



## [0.1.27]


### Bug Fixes
- Tests never write to the machine's sent log ([#72](https://github.com/creation-guidelines/lab-agent/pull/72)) ([f8ecc34])



## [0.1.26]


### Documentation
- How work is done here - the lab machine, land, reading messages before acknowledging ([#69](https://github.com/creation-guidelines/lab-agent/pull/69)) ([de0ef26])


### Features
- The status bar at the top of the board ([#71](https://github.com/creation-guidelines/lab-agent/pull/71)) ([de6dbbb])



## [0.1.25]


### Features
- A status bar with a quit button and the agent's state; times in local time ([#67](https://github.com/creation-guidelines/lab-agent/pull/67)) ([9c54284])



## [0.1.24]


### Features
- The pane under the mouse takes the focus; the wheel scrolls it ([#66](https://github.com/creation-guidelines/lab-agent/pull/66)) ([2f4055c])


### Testing
- The board keeps the newest message and the reply in view at every window size ([#64](https://github.com/creation-guidelines/lab-agent/pull/64)) ([35b5258])



## [0.1.23]


### Features
- The board measures text as it draws it; tasks open in full; the reply box grows freely ([#61](https://github.com/creation-guidelines/lab-agent/pull/61)) ([2b25514])
- Questions answerable on the board, and the agent's waits as a countdown ([#63](https://github.com/creation-guidelines/lab-agent/pull/63)) ([4ebec97])



## [0.1.22]


### Features
- The conversation no longer depends on ntfy ([#59](https://github.com/creation-guidelines/lab-agent/pull/59)) ([0ba077b])



## [0.1.21]


### Bug Fixes
- The Pages deploy tries a second time when GitHub's side fails ([#55](https://github.com/creation-guidelines/lab-agent/pull/55)) ([310d456])
- The environment's page link also when only the second deploy succeeds ([#57](https://github.com/creation-guidelines/lab-agent/pull/57)) ([8407178])


### Features
- The conversation goes through GitHub, not ntfy ([#58](https://github.com/creation-guidelines/lab-agent/pull/58)) ([2092e03])



## [0.1.20]


### Features
- Planned tasks and projects on the board; the chat keeps every reply ([#53](https://github.com/creation-guidelines/lab-agent/pull/53)) ([13f3bff])



## [0.1.19]


### Features
- Ask with buttons; popups only for what needs the user; the agent's messages in the board chat ([#51](https://github.com/creation-guidelines/lab-agent/pull/51)) ([809f155])



## [0.1.18]


### Features
- Desktop popups beside ntfy, and sent messages that show at once ([#50](https://github.com/creation-guidelines/lab-agent/pull/50)) ([3d4686f])



## [0.1.17]


### Bug Fixes
- The reply box grows and wraps instead of hiding what is typed ([#46](https://github.com/creation-guidelines/lab-agent/pull/46)) ([257ba96])



## [0.1.16]


### Features
- The control board - tasks, jobs and the conversation in one live view, with a reply box ([#44](https://github.com/creation-guidelines/lab-agent/pull/44)) ([15c82c6])



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


