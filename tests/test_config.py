#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for lab_agent/config.py. Plain asserts: `python3 tests/test_config.py`."""
import os
import sys
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from lab_agent.config import Config, ConfigError, from_dict  # noqa: E402


def test_home_defaults_under_the_real_users_home_not_a_root_only_system_path():
    # the actual bug: this field once defaulted to /var/lib/lab-agent, which only a root-owned system service could write
    # to; connect.sh runs the agent as an ordinary user, and the default silently pointed somewhere it had no permission to
    # create. Never again: the default must resolve under the real $HOME, and must never be a bare /var/... path.
    c = Config(jobs_remote="https://example/jobs.git", results_remote="https://example/results.git")
    assert c.home.startswith(os.path.expanduser("~") + os.sep), c.home
    assert not c.home.startswith("/var/"), f"defaulted to a system path an ordinary user cannot write to: {c.home}"


def test_jobs_and_results_remote_are_required():
    try:
        from_dict({}, "test")
    except ConfigError as error:
        assert "jobs remote is required" in str(error) or "jobs_remote" in str(error)
        return
    raise AssertionError("should have refused with neither remote set")


def test_jobs_and_results_remote_must_not_be_the_same():
    try:
        from_dict({"jobs": {"remote": "https://x/r.git"}, "results": {"remote": "https://x/r.git"}}, "test")
    except ConfigError as error:
        assert "must differ" in str(error)
        return
    raise AssertionError("should have refused when jobs and results point at the same repository")


def test_an_unknown_setting_is_refused_by_name():
    try:
        from_dict({"jobs": {"remote": "a"}, "results": {"remote": "b"}, "nonsense": 1}, "test")
    except ConfigError as error:
        assert "nonsense" in str(error)
        return
    raise AssertionError("should have refused the unknown setting")


def test_a_wrongly_typed_setting_is_refused():
    try:
        from_dict({"jobs": {"remote": "a"}, "results": {"remote": "b"}, "poll_seconds": "soon"}, "test")
    except ConfigError as error:
        assert "poll_seconds" in str(error)
        return
    raise AssertionError("should have refused a text value where a number was expected")


def test_a_non_positive_interval_is_refused():
    for field in ("poll_seconds", "push_seconds", "grace_seconds"):
        try:
            from_dict({"jobs": {"remote": "a"}, "results": {"remote": "b"}, field: 0}, "test")
        except ConfigError as error:
            assert field in str(error), (field, error)
            continue
        raise AssertionError(f"should have refused {field}=0")


def test_default_timeout_above_max_timeout_is_refused():
    try:
        from_dict({"jobs": {"remote": "a"}, "results": {"remote": "b"}, "default_timeout": 999, "max_timeout": 10}, "test")
    except ConfigError as error:
        assert "default_timeout" in str(error)
        return
    raise AssertionError("should have refused default_timeout above max_timeout")


def test_a_valid_minimal_config_loads_with_everything_else_defaulted():
    c = from_dict({"jobs": {"remote": "https://x/jobs.git"}, "results": {"remote": "https://x/results.git"}}, "test")
    assert c.jobs_remote == "https://x/jobs.git" and c.results_remote == "https://x/results.git"
    assert c.jobs_branch == "main" and c.results_branch == "results"


def test_a_flat_setting_outside_the_jobs_results_sections_is_read_as_is():
    c = from_dict({"jobs": {"remote": "a"}, "results": {"remote": "b"}, "poll_seconds": 2.5, "home": "/tmp/somewhere"}, "test")
    assert c.poll_seconds == 2.5 and c.home == "/tmp/somewhere"


def test_extra_env_must_be_a_table_and_is_passed_through():
    c = from_dict({"jobs": {"remote": "a"}, "results": {"remote": "b"}, "extra_env": {"PHONE_SERIAL": "abc"}}, "test")
    assert c.extra_env == {"PHONE_SERIAL": "abc"}
    try:
        from_dict({"jobs": {"remote": "a"}, "results": {"remote": "b"}, "extra_env": "not a table"}, "test")
    except ConfigError as error:
        assert "extra_env" in str(error)
        return
    raise AssertionError("should have refused a non-table extra_env")


TESTS = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]

if __name__ == "__main__":
    failed = 0
    for t in TESTS:
        try:
            t()
            print("ok  ", t.__name__)
        except Exception:
            failed += 1
            print("FAIL", t.__name__)
            traceback.print_exc()
    print(f"{len(TESTS) - failed}/{len(TESTS)} passed")
    sys.exit(1 if failed else 0)
