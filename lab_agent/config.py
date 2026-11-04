# SPDX-License-Identifier: GPL-3.0-or-later
"""The agent's settings: which repositories to read jobs from and report to, where it keeps its files, and how often it looks and pushes.

A TOML file (``/etc/lab-agent/config.toml`` by default); every setting but the two repositories has a default, and an unknown or ill-typed setting stops the
agent at start with a message that names it, instead of surprising it later.
"""
import os
import tomllib
from dataclasses import dataclass, field, fields

DEFAULT_PATH = "/etc/lab-agent/config.toml"


class ConfigError(Exception):
    """A setting is missing, unknown or of the wrong kind."""


@dataclass
class Config:
    jobs_remote: str                      # where jobs are read from (a git URL)
    results_remote: str                   # where progress is reported to (a different repository: the agent never writes where it reads)
    jobs_branch: str = "main"
    results_branch: str = "results"
    jobs_dir: str = "jobs"                # the directory in the jobs repository that holds the job scripts
    home: str = "/var/lib/lab-agent"      # the agent's own files: state, clones, the working directory of jobs
    poll_seconds: float = 5.0             # how often to look for new jobs
    push_seconds: float = 15.0            # how often to push the running job's log
    idle_heartbeat_seconds: float = 60.0  # how often an idle agent shows it is alive
    default_timeout: int = 1800           # seconds a job may run unless its header says otherwise
    max_timeout: int = 14400              # the most a job's header may ask for
    grace_seconds: float = 10.0           # between asking a job to stop and killing it
    log_chunk_bytes: int = 262144         # a job's log is pushed in files this big, so a push only sends the end
    max_pushed_log_bytes: int = 20971520  # per job; beyond it only the tail is pushed (the whole log stays on the machine)
    keep_results: int = 50                # job directories kept in the results repository; older ones are kept only on the machine
    git_timeout: int = 90                 # seconds any single git command may take
    ssh_key_jobs: str = ""                # read-only deploy key for the jobs repository (path), if it is reached over ssh
    ssh_key_results: str = ""             # deploy key with write access to the results repository only
    extra_env: dict = field(default_factory=dict)  # added to the environment of every job

    @property
    def state_path(self):
        return os.path.join(self.home, "state.json")

    @property
    def jobs_clone(self):
        return os.path.join(self.home, "jobs-repo")

    @property
    def results_clone(self):
        return os.path.join(self.home, "results-repo")

    @property
    def workdir(self):
        return os.path.join(self.home, "work")

    @property
    def archive_dir(self):
        return os.path.join(self.home, "archive")

    @property
    def local_logs(self):
        return os.path.join(self.home, "logs")


def load(path=DEFAULT_PATH):
    try:
        with open(path, "rb") as f:
            raw = tomllib.load(f)
    except FileNotFoundError:
        raise ConfigError(f"no configuration at {path}") from None
    except tomllib.TOMLDecodeError as error:
        raise ConfigError(f"{path} is not valid TOML: {error}") from error
    return from_dict(raw, path)


def from_dict(raw, where="the configuration"):
    flat = {}
    for section, values in raw.items():
        if isinstance(values, dict) and section != "extra_env":
            flat.update({f"{section}_{k}" if section in ("jobs", "results") and k in ("remote", "branch", "dir") else k: v for k, v in values.items()})
        else:
            flat[section] = values
    known = {f.name: f for f in fields(Config)}
    for name in flat:
        if name not in known:
            raise ConfigError(f"{where}: unknown setting {name!r}; known: {', '.join(sorted(known))}")
    for required in ("jobs_remote", "results_remote"):
        if not flat.get(required):
            raise ConfigError(f"{where}: {required.replace('_', ' ')} is required (put `remote = ...` under [{required.split('_')[0]}])")
    for name, value in flat.items():
        has_plain_default = known[name].default is not None and not isinstance(known[name].default, type(None))
        want = type(known[name].default) if has_plain_default and name != "extra_env" else None
        if name == "extra_env":
            if not isinstance(value, dict):
                raise ConfigError(f"{where}: extra_env must be a table")
        elif want in (int, float) and (isinstance(value, bool) or not isinstance(value, (int, float))):
            raise ConfigError(f"{where}: {name} must be a number, not {value!r}")
        elif want is str and not isinstance(value, str):
            raise ConfigError(f"{where}: {name} must be text, not {value!r}")
    config = Config(**flat)
    if config.jobs_remote == config.results_remote:
        raise ConfigError(f"{where}: the jobs and results repositories must differ: the agent never writes where it reads")
    for name in ("poll_seconds", "push_seconds", "idle_heartbeat_seconds", "grace_seconds"):
        if getattr(config, name) <= 0:
            raise ConfigError(f"{where}: {name} must be above zero")
    if config.default_timeout > config.max_timeout:
        raise ConfigError(f"{where}: default_timeout is above max_timeout")
    return config
