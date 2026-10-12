"""Shared pytest configuration.

Argon2 runs with its cheapest parameters during tests; production keeps the
RFC 9106 defaults. Parameters are stored in each hash, so a hash made under
one setting still verifies under the other.
"""

import os

os.environ.setdefault("ACTIVSYNC_ARGON2_TEST_PARAMS", "1")
