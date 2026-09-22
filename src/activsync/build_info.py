"""Which build is running.

Release images carry the version release-please wrote into `__version__`. Dev
images are built from a branch and get their tag name instead, so the footer
names the build you are actually running rather than the last release it was
branched from.
"""
import os

from activsync import __version__

BUILD_ENV_VAR = "ACTIVSYNC_BUILD"


def display_version() -> str:
    build = os.environ.get(BUILD_ENV_VAR, "").strip()
    return build or __version__


def is_release() -> bool:
    return display_version() == __version__

