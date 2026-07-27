import ast
import re
from pathlib import Path


PROJECT_ROOT = Path(__file__).parents[1]
PACKAGE_ROOT = PROJECT_ROOT / "src" / "activsync"


def _package_data_patterns() -> list[str]:
    pyproject = (PROJECT_ROOT / "pyproject.toml").read_text()
    match = re.search(r"^activsync\s*=\s*(\[[^\n]+\])$", pyproject, re.MULTILINE)
    assert match, "pyproject.toml is missing ActivSync's package-data declaration"
    return ast.literal_eval(match.group(1))


def test_runtime_icons_are_declared_as_package_data():
    patterns = _package_data_patterns()
    assert "static/favicon.png" in patterns
    assert "static/apple-touch-icon.png" in patterns
    assert (PACKAGE_ROOT / "static" / "favicon.png").is_file()
    assert (PACKAGE_ROOT / "static" / "apple-touch-icon.png").is_file()


def test_react_build_output_is_declared_as_package_data():
    patterns = _package_data_patterns()
    assert "web/*" in patterns
    assert "web/assets/*" in patterns
