// U6a line-break corpus, written by tools/gen_linebreak_fixtures.py.
#set page(width: 280pt, height: auto, margin: 0pt)
#set par(justify: true)
#set text(size: 11pt, lang: "en", hyphenate: false)
Configuration files often mix several kinds of values: numbers such as 1024 or 3.75, identifiers like max-connections, quoted strings, and paths (for example /usr/local/share). A well-behaved parser reports the line and column of every error, explains what it expected, and suggests a fix where one is obvious. It should never guess silently. Long, unbroken tokens -- hash digests, base64 blobs, or addresses -- are the hardest case for any layout engine, because they offer few places to break and tend to leave the line before them loose. Engineers who write documentation learn to wrap such tokens in code blocks, where a horizontal scroll is acceptable, instead of letting them stretch a paragraph.
