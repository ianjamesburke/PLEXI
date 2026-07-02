"""The session protocol the parent follows while playing the user.

The parent is a faithful proxy for a non-technical user. The rules are
deliberately restrictive so the run measures the authoring DX, not the parent's
expertise:

  1. Open with the fixture's initial prompt verbatim. Nothing else.
  2. When the child asks a question, answer only with user-level knowledge —
     match the question to a fixture ``answers`` intent. Never name a command,
     file path, or SDK symbol; never suggest an implementation.
  3. If the child asks something no user would know how to answer, say so
     ("I don't know, you're the expert") and record it as friction.
  4. Record every intervention (prompt, answer, nudge) as an observation.

Question->intent matching is intentionally shallow (keyword overlap): a real user
does not parse intent precisely either, and a wrong-but-plausible answer is itself
signal about prompt ambiguity.
"""

from __future__ import annotations

from dataclasses import dataclass

from .config import Fixture

# Said when the child asks something outside user-level knowledge.
NO_USER_KNOWLEDGE = "I don't know how that works — you're the one building it. Do what seems best."


@dataclass
class Intervention:
    turn: int
    kind: str  # "prompt" | "answer" | "no-knowledge" | "nudge"
    question: str | None
    text: str


class SessionProtocol:
    def __init__(self, fixture: Fixture) -> None:
        self.fixture = fixture
        self.turn = 0
        self.interventions: list[Intervention] = []

    def initial_prompt(self) -> Intervention:
        self.turn += 1
        iv = Intervention(self.turn, "prompt", None, self.fixture.prompt)
        self.interventions.append(iv)
        return iv

    def answer(self, question: str) -> Intervention:
        """Answer a child question with user-level knowledge only."""
        self.turn += 1
        intent = self._match_intent(question)
        if intent is None:
            iv = Intervention(self.turn, "no-knowledge", question, NO_USER_KNOWLEDGE)
        else:
            iv = Intervention(self.turn, "answer", question, self.fixture.answers[intent])
        self.interventions.append(iv)
        return iv

    def _match_intent(self, question: str) -> str | None:
        q = question.lower()
        best: tuple[int, str] | None = None
        for intent in self.fixture.answers:
            score = sum(1 for tok in intent.lower().split("_") if tok in q)
            if score and (best is None or score > best[0]):
                best = (score, intent)
        return best[1] if best else None
