#!/usr/bin/env python3
"""Writes the U6a line-break corpus, tests/fixtures/eval/linebreak/*.typ.

Each fixture is one paragraph on a page as wide as its measure, with no margin and an automatic height, so
the lines Typst sets are the lines a paragraph of that measure breaks into. The prose is original to this
corpus, or Typst's own lorem(). Rerun after editing and commit the output; the expected lines are never
stored, since the test asks typst for them each time.
"""

import os

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "tests", "fixtures", "eval", "linebreak")

EN = {
    "craft": (
        "Printing began as a craft of patience. A compositor set each line by hand, choosing where to "
        "break a word and how much space to leave between the others, and the quality of a page depended "
        "on judgements made thousands of times a day. When machines took over the setting of type, the "
        "judgements did not disappear; they moved into rules. Some rules were simple: never end a line "
        "with a single short word, avoid two hyphens in a row, keep the spaces even. Others weighed one "
        "line against the next, looking ahead to find a set of breaks that made the whole paragraph "
        "comfortable to read. The best of these methods, published in 1981, treated the paragraph as a "
        "single problem rather than a sequence of separate choices."
    ),
    "config": (
        "Configuration files often mix several kinds of values: numbers such as 1024 or 3.75, identifiers "
        "like max-connections, quoted strings, and paths (for example /usr/local/share). A well-behaved "
        "parser reports the line and column of every error, explains what it expected, and suggests a fix "
        "where one is obvious. It should never guess silently. Long, unbroken tokens -- hash digests, "
        "base64 blobs, or addresses -- are the hardest case for any layout engine, because they offer few "
        "places to break and tend to leave the line before them loose. Engineers who write documentation "
        "learn to wrap such tokens in code blocks, where a horizontal scroll is acceptable, instead of "
        "letting them stretch a paragraph."
    ),
    "ferry": (
        "The ferry left at seven, though nobody on the pier seemed to believe it. \"It's always late,\" "
        "said the woman selling coffee, handing over a paper cup that was too hot to hold. The harbour "
        "smelled of diesel and salt; gulls argued over a crust near the ticket office. When the horn "
        "finally sounded, a dozen passengers who had been sheltering under the awning hurried down the "
        "ramp, shoes clattering on the wet steel. Out beyond the breakwater the water turned from grey to "
        "green, and the island appeared as a low, dark line that grew, over forty minutes, into cliffs, "
        "fields, a church spire, and a row of white houses facing the sea."
    ),
    "long": (
        "Comprehensive documentation of internationalisation requirements, accessibility considerations, "
        "and interoperability constraints distinguishes professional engineering organisations from their "
        "less disciplined counterparts. Responsibilities are allocated unambiguously; characteristics of "
        "the underlying infrastructure are recorded systematically; and incompatibilities between "
        "successive implementations are identified before deployment rather than afterwards. "
        "Nevertheless, overcomplicated specifications can themselves become counterproductive, "
        "discouraging contributors and encouraging undocumented workarounds that undermine "
        "maintainability."
    ),
    "lorem": None,
}

DE = {
    "faehre": (
        "Die Fähre legte um sieben Uhr ab, obwohl niemand am Anleger so recht daran glauben wollte. Der "
        "Hafen roch nach Diesel und Salz, und die Möwen stritten sich um ein Stück Brot neben dem "
        "Fahrkartenschalter. Als endlich das Horn ertönte, eilten ein Dutzend Fahrgäste die Rampe hinunter, "
        "die Schuhe klapperten auf dem nassen Stahl. Hinter der Mole wurde das Wasser grün, und die Insel "
        "erschien als niedrige, dunkle Linie, die in vierzig Minuten zu Klippen, Feldern, einem Kirchturm "
        "und einer Reihe weißer Häuser wurde."
    ),
    "doku": (
        "Eine sorgfältige Dokumentation der Anforderungen an Barrierefreiheit, Internationalisierung und "
        "Zusammenarbeit unterscheidet professionelle Entwicklungsabteilungen von weniger disziplinierten. "
        "Zuständigkeiten werden eindeutig verteilt, Eigenschaften der Infrastruktur werden systematisch "
        "erfasst, und Unverträglichkeiten zwischen aufeinanderfolgenden Versionen werden vor der "
        "Auslieferung erkannt."
    ),
}

MEASURES = [180, 205, 230, 255, 280, 305, 330, 355, 380, 405, 430, 460]
SIZES = [10, 11, 12]


def body(src, text, n):
    if text is None:
        return "#lorem(%d)" % n
    return text


def write(name, width, size, lang, par, text_args, content):
    lines = ["// U6a line-break corpus, written by tools/gen_linebreak_fixtures.py.",
             "#set page(width: %dpt, height: auto, margin: 0pt)" % width]
    if par:
        lines.append("#set par(%s)" % par)
    args = ["size: %dpt" % size, 'lang: "%s"' % lang] + text_args
    lines.append("#set text(%s)" % ", ".join(args))
    lines.append(content)
    with open(os.path.join(OUT, name + ".typ"), "w") as f:
        f.write("\n".join(lines) + "\n")


def main():
    os.makedirs(OUT, exist_ok=True)
    for f in os.listdir(OUT):
        if f.endswith(".typ"):
            os.remove(os.path.join(OUT, f))
    en_keys = list(EN.keys())
    count = 0
    for i, w in enumerate(MEASURES):
        size = SIZES[i % 3]
        variants = [
            ("j", "justify: true", []),
            ("r", "justify: false", []),
            ("jn", "justify: true", ["hyphenate: false"]),
            ("rh", "justify: false", ["hyphenate: true"]),
        ]
        for v, (tag, par, targs) in enumerate(variants):
            key = en_keys[(i + v) % len(en_keys)]
            write("en-%s-%d-%s" % (key, w, tag), w, size, "en", par, targs, body(key, EN[key], 90 + 7 * i))
            count += 1
    for i, w in enumerate(MEASURES[::2]):
        size = SIZES[(i + 1) % 3]
        for key in DE:
            for tag, par in [("r", "justify: false"), ("jn", "justify: true")]:
                write("de-%s-%d-%s" % (key, w, tag), w, size, "de", par, ["hyphenate: false"], DE[key])
                count += 1
    costs = [
        ("hyph50", "costs: (hyphenation: 50%)"),
        ("hyph200", "costs: (hyphenation: 200%)"),
        ("runt0", "costs: (runt: 0%)"),
        ("runt300", "costs: (runt: 300%)"),
        ("both", "costs: (hyphenation: 20%, runt: 400%)"),
        ("hyph0", "costs: (hyphenation: 0%)"),
    ]
    for i, (tag, c) in enumerate(costs):
        w = MEASURES[(2 * i + 1) % len(MEASURES)]
        key = en_keys[i % len(en_keys)]
        write("en-%s-%d-cost-%s" % (key, w, tag), w, 11, "en", "justify: true", [c], body(key, EN[key], 120))
        count += 1
    modes = [
        ("js", 'justify: true, linebreaks: "simple"'),
        ("ro", 'justify: false, linebreaks: "optimized"'),
    ]
    for i, w in enumerate([200, 290, 390]):
        for tag, par in modes:
            key = en_keys[(i + 2) % len(en_keys)]
            write("en-%s-%d-%s" % (key, w, tag), w, 11, "en", par, [], body(key, EN[key], 110))
            count += 1
    print("wrote %d fixtures to %s" % (count, os.path.normpath(OUT)))


if __name__ == "__main__":
    main()
