# Edit Jira text as Markdown with raw wiki pieces

Jira Data Center stores descriptions and comments as wiki markup. A full conversion to Markdown and back changed 428 of 1452 lines in real Tickets. Thus jui shows text as Markdown, but it keeps the elements that have no clean Markdown form (panels, colors, images with attributes, attachment links, macros) as the original wiki text. These Raw wiki pieces are fenced `jira-wiki` blocks and `jira:` code spans. jui sends a text only when the user changed it, and it shows a diff before it changes existing text. Do not "simplify" this to a full Markdown round trip. That damages the text of other users.

## Considered options

- Full Markdown round trip with `amdc`: rejected, because it changes the text.
- Raw wiki markup only: rejected, because the user wants to write and read Markdown.
- The `amdc` escape hatch (ADF JSON payloads): rejected, because a person cannot read the payloads in an editor.
