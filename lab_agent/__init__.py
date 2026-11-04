# SPDX-License-Identifier: GPL-3.0-or-later
"""A pull-based agent for a lab machine: it runs the scripts it finds in a git repository, one at a time, and reports progress back through git.

Nothing connects *to* the machine. The agent looks at the jobs repository every few seconds, runs each new job script once, in order, and writes
the log of the running job into a results repository about every fifteen seconds. Whoever writes the jobs (a person, or an agent such as Claude)
reads the results from there and writes the next job.
"""
__version__ = "0.1.0"
