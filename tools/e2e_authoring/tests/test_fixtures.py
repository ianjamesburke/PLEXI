"""The committed prompt library must stay loadable and user-realistic.

Every fixture is a user speaking — so the prompt and answers must never leak an
implementation hint (a command, file path, or SDK symbol). This guards the
parent/child contract at the corpus level, not just in one fixture.
"""

from pathlib import Path

import pytest

from plexi_e2e.config import Fixture

FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"
ALL = sorted(FIXTURES.glob("*.toml"))

# Words a user would never say — they name an implementation, not an intent.
FORBIDDEN = (
    "plexi", "manifest", "sdk", "capability", "capabilities", "pgap", "toml",
    "python", "def ", "import ", "cli", "command", "terminal", "widget",
    ".py", "main.py", "app init", "render", "l1", "socket",
)

GRADES = {"easy", "medium", "hard"}


def test_library_is_non_empty():
    assert ALL, "no fixtures found in fixtures/"


@pytest.mark.parametrize("path", ALL, ids=lambda p: p.stem)
def test_fixture_loads_and_is_graded(path):
    fx = Fixture.load(path)
    assert fx.id
    assert fx.difficulty in GRADES
    assert fx.description
    assert fx.prompt
    assert fx.answers, "a fixture needs user-level answers for the parent to give"


@pytest.mark.parametrize("path", ALL, ids=lambda p: p.stem)
def test_prompt_and_answers_have_no_implementation_hints(path):
    fx = Fixture.load(path)
    haystack = (fx.prompt + " " + " ".join(fx.answers.values())).lower()
    leaked = [w for w in FORBIDDEN if w in haystack]
    assert not leaked, f"{path.name} prompt/answers leak implementation hints: {leaked}"


def test_expected_difficulty_spread():
    grades = {Fixture.load(p).difficulty for p in ALL}
    # The library must span the difficulty range, not cluster at one grade.
    assert {"easy", "medium", "hard"} <= grades
