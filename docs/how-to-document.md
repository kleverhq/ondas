# Documentation Rules

Follow the spirit of Unix when working with the documentation in docs/ and comments:

- document only things and surprises that would surprise an experienced engineer that knows all about Rust, HDL EDA, RTL waveform formats and building public libraries like that.
- When asked to update docs/ and comments - bring them in sync with the project, removing stale things, updating surprises and removing obvious things.
- Prefer descriptive filenames for files in docs/, for they are more explanatory when used with tree docs
- Assume that the reader knows how to use grep and follow the links. E.g. instead of documenting makefile step in markdown, just mention it, same with the code.
- Remember that full doc tree is always loaded in the context at the start of each session. So in instructions  refer to repository documents by filename in backticks when unambiguous in the doc tree, e.g. prototype.md. Otherwise use a repo-root-relative path without a leading slash, e.g. docs/product/concepts.md. Avoid Markdown links and ../ paths for these references.
- no manual wrapping of lines in MD files.
- no tables (unless asked)
