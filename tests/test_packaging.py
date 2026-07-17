import ast
import re
from pathlib import Path, PurePosixPath


PROJECT_ROOT = Path(__file__).parents[1]
PACKAGE_ROOT = PROJECT_ROOT / "src" / "activsync"


def _package_data_patterns() -> list[str]:
    pyproject = (PROJECT_ROOT / "pyproject.toml").read_text()
    match = re.search(r"^activsync\s*=\s*(\[[^\n]+\])$", pyproject, re.MULTILINE)
    assert match, "pyproject.toml is missing ActivSync's package-data declaration"
    return ast.literal_eval(match.group(1))


def test_all_template_assets_are_included_in_package_data():
    patterns = _package_data_patterns()
    template_assets = (
        path.relative_to(PACKAGE_ROOT).as_posix()
        for path in (PACKAGE_ROOT / "templates").rglob("*")
        if path.is_file()
    )

    missing = [
        asset
        for asset in template_assets
        if not any(PurePosixPath(asset).match(pattern) for pattern in patterns)
    ]

    assert missing == []
