from dataclasses import dataclass


@dataclass
class Level:
    id: int
    name: str
    text: str
    time_sec: int
    # cumulative stars needed to unlock (0 = always unlocked)
    stars_needed: int


LEVELS: list[Level] = [
    Level(
        id=1,
        name="Home Row Intro",
        text="a s d f j k l ; a f j ; d k s l g h a s d f g h j k l ; f j d k s l",
        time_sec=30,
        stars_needed=0,
    ),
    Level(
        id=2,
        name="Home Row Words",
        text="ash has flag gash half shall flash glad glass lass dash falls hall lads",
        time_sec=45,
        stars_needed=0,
    ),
    Level(
        id=3,
        name="Home Row Phrases",
        text="a glad lad shall fall; a lass dashes a flag; glass halls shall flash",
        time_sec=60,
        stars_needed=2,
    ),
    Level(
        id=4,
        name="Home Row Sentences",
        text="all lads shall fall; a glad flask shall gash a glass hall; flash a flag",
        time_sec=75,
        stars_needed=5,
    ),
    Level(
        id=5,
        name="Home Row Speed",
        text="shall a glad lad dash; glass halls fall; ash flask; flash flags; a lass gashes; half shall fall; lads dash; gash a glass; glad halls shall flash a flag",
        time_sec=90,
        stars_needed=10,
    ),
    Level(
        id=6,
        name="Top Row Joins",
        text="type quit power write upper quite tower poetry rupture wrote pure quote type write quit power",
        time_sec=75,
        stars_needed=15,
    ),
    Level(
        id=7,
        name="Full Alphabet",
        text="the quick brown fox jumps over a lazy dog; pack my box with five dozen liquor jugs; how vexingly quick daft zebras jump",
        time_sec=90,
        stars_needed=21,
    ),
    Level(
        id=8,
        name="Numbers",
        text="in 2026 there are 365 days; call 555-1234 for help; order 42 items at $9.99 each; the 10th floor has 8 rooms numbered 101 to 108",
        time_sec=90,
        stars_needed=27,
    ),
    Level(
        id=9,
        name="Punctuation",
        text="hello, world! how are you? i'm fine, thanks. let's type: 'fast' and (accurately). wow — that's great! don't stop now.",
        time_sec=90,
        stars_needed=34,
    ),
    Level(
        id=10,
        name="Grand Finale",
        text="typing mastery requires practice, patience, and precision. the quick brown fox — yes, that fox — jumps over 7 lazy dogs! can you type 'supercalifragilistic' without errors? good luck: you'll need it (and maybe coffee too).",
        time_sec=120,
        stars_needed=41,
    ),
]
