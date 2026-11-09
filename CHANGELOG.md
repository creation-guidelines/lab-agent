# Changelog

All notable changes to lab-agent are documented here.
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


