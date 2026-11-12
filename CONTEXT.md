# jui

jui is a personal Jira client for one user on one Jira Data Center server. It has a CLI and a TUI with the same functions.

## Language

### Jira objects

**Ticket**:
One Jira work item (Task, Bug, Story, Epic, Sub-task, AppSec, or other type).
_Avoid_: Issue (except in Jira API names), item, card

**Status**:
The workflow state of a Ticket, for example "In Progress".
_Avoid_: State, column

**Transition**:
A permitted move of a Ticket from one Status to a different Status.
_Avoid_: Status change, move

**Board**:
A Jira Agile board (Scrum or Kanban) that shows Tickets in columns.
_Avoid_: Project, view

**Sprint**:
A time box on a Scrum Board that contains Tickets.

**Epic**:
A Ticket that contains other Tickets as its children.

**Sub-task**:
A Ticket that is a child of one parent Ticket.

**Link**:
A typed relation between two Tickets, for example "blocks".
_Avoid_: Dependency, relation

**Worklog**:
A record of time that the user spent on a Ticket.
_Avoid_: Time entry, timesheet

### jui concepts

**My Tickets**:
The Tickets that the default JQL query in the Settings finds.
_Avoid_: Inbox, my issues

**My Boards**:
The Boards in the board list in the Settings. Only the user adds Boards to this list. jui does not use favourite Boards or Recent Boards from Jira as My Boards.
_Avoid_: All boards, favourite boards

**Template**:
A text file that jui opens in `$EDITOR` to create or edit a Ticket. It contains the fields of the Ticket and the description in Markdown.
_Avoid_: Form (a Form is the TUI popup, not the file)

**Create Form**:
The TUI popup that `n` and `N` open to create a Ticket without `$EDITOR`. It shows the summary, the required fields, the frequent fields, and a description box. Ctrl-E moves its values into a Template in `$EDITOR`. The same checks as for a Template apply.
_Avoid_: Wizard, dialog

**Draft**:
A saved copy of a Template that jui did not send to Jira, for example because the Template was not valid, or the user closed a Create Form with text in it. The user can use it again.
_Avoid_: Backup, recovery file

**Raw wiki piece**:
A part of a Ticket text that jui keeps as the original Jira wiki markup in the Markdown, because Markdown cannot show it.
_Avoid_: Escape hatch, raw block

**Settings**:
All user-configurable values of jui. The user can edit them in the config file or in the TUI Settings page.
_Avoid_: Preferences, options
