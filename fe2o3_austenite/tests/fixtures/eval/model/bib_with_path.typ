// A bibliography path given through `bibliography.with(..)` resolves at the file that wrote it, not at
// the template in a subdirectory that makes the call.
#import "tpl/lib.typ": template
#show: template.with(bibliography: bibliography.with("refs.bib"))
Cite @smith2020 and @brown2018.
