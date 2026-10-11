Typst 0.15.1 output for B2a's renderer tests. Each `<style>.typ` is compiled over `refs.bib` with
`typst compile --features html --format html <style>.typ`; the body is kept, one `<li>` per line,
and Typst's MathML for `$x^2$` is written back as `$x^2$`. The PDF route (`pdftotext -layout`)
gives the same text.

B2b extends `refs.bib` with six entries carrying names, dates and page ranges (n1-n6) and adds
american-medical-association, a numeric style with `second-field-align`. Typst writes a run of
two spaces as `<span style="white-space: pre-wrap">&#x20;&#x20;</span>`; the files hold the two
spaces themselves.
