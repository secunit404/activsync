"""Shared pytest configuration.

Keep PBKDF2 fast during tests; production uses the default high iteration count.
The value is stored alongside each hash, so verification still works either way.
"""

import os

os.environ.setdefault("ACTIVSYNC_PBKDF2_ITERATIONS", "1000")
