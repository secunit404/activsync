import re
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parent.parent / "scripts" / "dev-image-tag.sh"
DOCKER_TAG = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9._-]{0,127}$")


def run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(SCRIPT), *args], capture_output=True, text=True, check=False
    )


@pytest.mark.parametrize(
    ("branch", "expected"),
    [
        ("feat/hevy-react-frontend", "dev-feat-hevy-react-frontend"),
        ("fix/pin_clock.v2", "dev-fix-pin_clock.v2"),
        ("dependabot/github_actions/docker/x@6", "dev-dependabot-github_actions-docker-x-6"),
    ],
)
def test_branch_maps_to_prefixed_sanitized_tag(branch: str, expected: str) -> None:
    result = run(branch)

    assert result.returncode == 0
    assert result.stdout.strip() == expected


def test_long_branch_is_capped_at_docker_tag_limit() -> None:
    tag = run("feat/" + "x" * 300).stdout.strip()

    assert len(tag) == 128
    assert DOCKER_TAG.match(tag)


@pytest.mark.parametrize("args", [(), ("",)])
def test_missing_branch_is_rejected(args: tuple[str, ...]) -> None:
    result = run(*args)

    assert result.returncode == 2
    assert "usage" in result.stderr
