Typst 0.15.1 output for B2a's renderer tests. Each `<style>.typ` is compiled over `refs.bib` with
`typst compile --features html --format html <style>.typ`; the body is kept, one `<li>` per line,
and Typst's MathML for `$x^2$` is written back as `$x^2$`. The PDF route (`pdftotext -layout`)
gives the same text.
