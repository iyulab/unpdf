# Test fixtures

Real documents the test suite runs against. They are committed so that every test runs
everywhere: a test must fail, not skip, when its fixture is missing.

| File | What it is | Why it is here |
|---|---|---|
| `latex_resume.pdf` | A two-page résumé typeset with pdfTeX (EB Garamond, Type 1 fonts with declared widths) | Section headers in bold capitals, bold/italic runs sharing a row, right-aligned dates set with `\hfill`, bullet lists, hard hyphens |

## Provenance and licence

`latex_resume.pdf` was contributed for this test suite by its author, who agreed to its
inclusion under this repository's MIT licence. Every personal detail in it is a
placeholder (name, address, contact details, employers, body text).
