# Issue tracker: Local Markdown

Issues and specs for this repository are markdown files in `.scratch/`.

## Conventions

- Use one directory for each feature: `.scratch/<feature-slug>/`.
- The spec is `.scratch/<feature-slug>/spec.md`.
- Each ticket is one file: `.scratch/<feature-slug>/issues/<NN>-<slug>.md`. The numbers start at `01`. Do not put many tickets in one file.
- A `Status:` line near the top of each file shows the state of the ticket.
- Put comments at the bottom of the file, below a `## Comments` heading.

## Wayfinding operations

Used by `/wayfinder`. The map is one file. Each ticket is one child file.

- **Map**: `.scratch/<effort>/map.md`.
- **Child ticket**: `.scratch/<effort>/issues/NN-<slug>.md`. A `Type:` line shows the ticket type (`research`, `prototype`, `grilling`, or `task`). A `Status:` line shows `open`, `claimed`, or `resolved`.
- **Blocking**: a `Blocked by: NN, NN` line near the top. A ticket is unblocked when all the files in that line are `resolved`.
- **Frontier**: find the files in `.scratch/<effort>/issues/` that are open, unblocked, and not claimed. The lowest number is first.
- **Claim**: set `Status: claimed` and save the file before you start work.
- **Resolve**: write the answer below an `## Answer` heading. Set `Status: resolved`. Then add a context pointer (gist and link) to "Decisions so far" in `map.md`.
