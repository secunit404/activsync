import pytest

from activsync import __version__, build_info


@pytest.fixture(autouse=True)
def clear_build_env(monkeypatch):
    monkeypatch.delenv(build_info.BUILD_ENV_VAR, raising=False)


def test_unset_build_reports_the_released_version():
    assert build_info.display_version() == __version__
    assert build_info.is_release() is True


def test_build_matching_the_release_is_a_release(monkeypatch):
    monkeypatch.setenv(build_info.BUILD_ENV_VAR, __version__)

    assert build_info.display_version() == __version__
    assert build_info.is_release() is True


def test_branch_build_reports_its_own_name(monkeypatch):
    monkeypatch.setenv(build_info.BUILD_ENV_VAR, "dev-feat-hevy-react-frontend")

    assert build_info.display_version() == "dev-feat-hevy-react-frontend"
    assert build_info.is_release() is False


@pytest.mark.parametrize("value", ["", "   "])
def test_blank_build_falls_back_to_the_released_version(monkeypatch, value):
    monkeypatch.setenv(build_info.BUILD_ENV_VAR, value)

    assert build_info.display_version() == __version__
    assert build_info.is_release() is True

