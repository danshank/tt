# Motivation

Claude Code makes it practical to run several agent sessions at once. One session refactors a module, another investigates a bug, a third drafts a ticket. Each session holds its own context and finishes or stalls on its own schedule.

The person running them has no equivalent place to hold the whole picture. The work is spread across terminal tabs, tmux windows, and memory. Once more than a couple of sessions run at the same time, there is more open work than one person can track in their head. Half-finished tasks go unnoticed, and the person loses track of what they set out to do.

tt keeps that picture in one todo tree that the user owns. The user decides how the work is organized: which goals sit at the top, how each one breaks down, and what order the parts come in. The tree puts the overall picture first and the details of any one task second.

From the tree, the user moves into the details and back out quickly. Pressing `o` on a todo opens its Claude session, or starts a new one with the todo as the first prompt. Closing the session, or switching back to tt's tmux window, returns to the tree with the rest of the work in view.

As the tree fills up and more todos move into progress, it gets harder to see which ones are still being worked on. Finished work is easy to lose track of too: the tree hides older done todos, and the user forgets what they have already done. The in-progress view lists only todos that have a session, most recent first, so the user can see what is being worked on right now. The recently completed view lists finished todos, newest first, so the user can see the progress they have made.

The timer makes the user step back out at regular intervals. Deep in an implementation, it's easy to stop checking the work against the larger goal. When a work block ends (25 minutes by default), tt opens a reflection: a short note on what the user achieved in that block. tt sends the reflection and recent session activity to Claude, which suggests open todos that look finished, and the user decides which to check off. The user then returns to the work, having checked it against the overall picture.
