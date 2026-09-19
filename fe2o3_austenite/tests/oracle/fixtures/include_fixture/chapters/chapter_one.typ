// The "chapter" half of the cross-directory-include fixture: a chapter file, itself already reached by
// the root's own `#include "chapters/chapter_one.typ"`, that in turn includes a file from a SIBLING
// directory via a `../` relative path -- the exact shape that went unresolved in Lucronics.

This opening paragraph belongs to the chapter itself, before its own cross-directory include, so both the
chapter's own prose and the included evidence set in the one document-order flow rather than one
swallowing the other.

#include "../evidence/evidence_one.typ"

This closing paragraph belongs to the chapter again, after the included file's content, so the chapter
resumes its own flow past the include boundary rather than the boundary swallowing everything that follows
it too.
